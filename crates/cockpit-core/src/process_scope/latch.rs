//! Single-use private placement latch. A write receipt is not execution proof.
use std::{collections::BTreeMap, path::{Path, PathBuf}, time::Duration};
use cockpit_protocol::{orchestration::{AgentLaunchReceipt, RunKind}, projects::ScopeMode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use crate::InspectionError;

const MAX_PAYLOAD: usize = 16 * 1024;
const HEADER: usize = 8;
const CHECKSUM: usize = 32;
const MAX_FRAME: usize = HEADER + MAX_PAYLOAD + CHECKSUM;
const MAGIC: &[u8; 4] = b"CKL1";

#[derive(Debug)]
pub struct AgentLaunch {
    pub unit: Option<String>,
    pub latch: PathBuf,
    pub nonce: String,
    pub argv: Vec<String>,
    pub receipt: AgentLaunchReceipt,
}

/// Startup choice only. Neither variant is physical containment authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeStartup { Scoped, Uncontained }

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupReport { nonce: String, scope_entered: bool }

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload { nonce: String, env: BTreeMap<String, String> }

fn failure(code: &str, message: impl Into<String>) -> InspectionError {
    InspectionError::new(code, message)
}

fn allowed_environment(key: &str) -> bool {
    matches!(key, "COCKPIT_WORKSPACE_ID" | "COCKPIT_CONTEXT_PATH" | "COCKPIT_LIBRARY_ROOT"
        | "COCKPIT_REPOSITORY_KEY" | "COCKPIT_ARTIFACT_URL" | "COCKPIT_SOURCE_PATH"
        | "HERDR_WORKSPACE_ID" | "HERDR_TAB_ID" | "HERDR_PANE_ID")
}

fn validate_environment(env: &BTreeMap<String, String>) -> Result<(), InspectionError> {
    if env.len() > 9 || env.iter().any(|(key, value)| !allowed_environment(key) || value.contains('\0')) {
        return Err(failure("latch_payload_invalid", "placement overlay contains an unauthorized key or NUL"));
    }
    Ok(())
}

struct BoundedFrame(Vec<u8>);
impl std::io::Write for BoundedFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > (HEADER + MAX_PAYLOAD).saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("placement payload exceeds 16 KiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

fn encode(receipt: &AgentLaunchReceipt, env: &BTreeMap<String, String>) -> Result<Vec<u8>, InspectionError> {
    validate_environment(env)?;
    // Serialize borrowed fields: no extra environment or credential copies.
    #[derive(Serialize)]
    struct BorrowedPayload<'a> { nonce: &'a str, env: &'a BTreeMap<String, String> }
    let mut frame = BoundedFrame(Vec::with_capacity(MAX_FRAME));
    frame.0.extend_from_slice(MAGIC);
    frame.0.extend_from_slice(&[0; 4]);
    serde_json::to_writer(&mut frame, &BorrowedPayload { nonce: &receipt.nonce, env })
        .map_err(|e| failure("latch_payload_invalid", e.to_string()))?;
    let size = frame.0.len() - HEADER;
    frame.0[4..HEADER].copy_from_slice(&(size as u32).to_be_bytes());
    let checksum = Sha256::digest(&frame.0[HEADER..]);
    frame.0.extend_from_slice(&checksum);
    Ok(frame.0)
}

fn frame_size(header: &[u8]) -> Result<usize, InspectionError> {
    if header.len() < HEADER || &header[..4] != MAGIC {
        return Err(failure("latch_payload_invalid", "invalid placement frame header"));
    }
    let size = u32::from_be_bytes(header[4..HEADER].try_into().expect("four-byte length")) as usize;
    if size == 0 || size > MAX_PAYLOAD {
        return Err(failure("latch_payload_invalid", "invalid placement frame length"));
    }
    Ok(HEADER + size + CHECKSUM)
}

fn decode(frame: &[u8], nonce: &str) -> Result<BTreeMap<String, String>, InspectionError> {
    let size = frame_size(frame)?;
    if frame.len() != size { return Err(failure("latch_payload_invalid", "incomplete or extra placement frame data")); }
    let payload_end = size - CHECKSUM;
    let payload = &frame[HEADER..payload_end];
    if &Sha256::digest(payload)[..] != &frame[payload_end..] {
        return Err(failure("latch_payload_invalid", "placement frame checksum mismatch"));
    }
    let payload: Payload = serde_json::from_slice(payload).map_err(|e| failure("latch_payload_invalid", e.to_string()))?;
    if payload.nonce != nonce { return Err(failure("latch_payload_invalid", "placement nonce mismatch")); }
    validate_environment(&payload.env)?;
    Ok(payload.env)
}

/// `base` is the configured state root, not its orchestration subdirectory.
/// Journal this receipt before opening a terminal. Each call creates a fresh
/// FIFO; abandoned attempts are never reused or retrospectively authorized.
pub fn agent_launch(
    cli_path: &Path, base: &Path, store_identity: &str, run_id: &str, kind: &RunKind,
    epoch: u64, description: &str, scope: ScopeMode, argv: Vec<String>,
) -> Result<AgentLaunch, InspectionError> {
    if argv.is_empty() || !cli_path.is_absolute() || cli_path.to_str().is_none() {
        return Err(failure("latch_launch_invalid", "absolute Cockpit CLI and nonempty literal harness argv required"));
    }
    #[cfg(unix)]
    {
        let receipt = unix::create(base, run_id, epoch)?;
        let latch = PathBuf::from(&receipt.latch);
        #[cfg(target_os = "linux")]
        let unit = (scope == ScopeMode::Auto).then(||
            super::scope_unit(store_identity, run_id, kind, epoch));
        #[cfg(not(target_os = "linux"))]
        let unit: Option<String> = { let _ = (scope, store_identity, kind); None };
        let mut wrapped = Vec::with_capacity(argv.len() + 16);
        wrapped.extend([cli_path.to_string_lossy().into_owned(), "agent-exec".into()]);
        if let Some(unit) = &unit { wrapped.extend(["--unit".into(), unit.clone()]); }
        wrapped.extend(["--description".into(), description.into(), "--latch".into(), receipt.latch.clone(),
            "--nonce".into(), receipt.nonce.clone(), "--latch-device".into(), receipt.latch_device.clone(),
            "--latch-inode".into(), receipt.latch_inode.clone(), "--".into()]);
        wrapped.extend(argv);
        Ok(AgentLaunch { unit, latch, nonce: receipt.nonce.clone(), argv: wrapped, receipt })
    }
    #[cfg(not(unix))]
    { let _ = (base, store_identity, run_id, kind, epoch, description, scope); Err(failure("latch_unsupported", "private FIFO placement requires a Unix host")) }
}

/// Pre-move readiness only: open the exact FIFO without writing any bytes.
/// Phase two opens its reader only after scope entry or explicit fallback.
/// Scheduler callers bound this startup wait; it is not an idle polling source.
pub fn latch_ready(receipt: &AgentLaunchReceipt) -> Result<bool, InspectionError> {
    #[cfg(unix)]
    { unix::ready(receipt) }
    #[cfg(not(unix))]
    { let _ = receipt; Err(failure("latch_unsupported", "private FIFO placement requires a Unix host")) }
}

/// Read the private phase-two outcome only while the exact FIFO reader is
/// ready. Scoped still requires kernel capture; absence is never fallback.
pub fn latch_startup(receipt: &AgentLaunchReceipt) -> Result<Option<ScopeStartup>, InspectionError> {
    #[cfg(unix)]
    { unix::startup(receipt) }
    #[cfg(not(unix))]
    { let _ = receipt; Err(failure("latch_unsupported", "private FIFO placement requires a Unix host")) }
}

/// One framed nonblocking write, never write_all or retries. The caller must
/// persist a release intent first. ENXIO means no reader/no bytes; all uncertain
/// or partial writes are Unknown and MUST NOT be blindly replayed.
pub fn release_latch(receipt: &AgentLaunchReceipt, env: &BTreeMap<String, String>) -> Result<(), InspectionError> {
    let frame = encode(receipt, env)?;
    #[cfg(unix)]
    { unix::release(receipt, &frame) }
    #[cfg(not(unix))]
    { let _ = frame; Err(failure("latch_unsupported", "private FIFO placement requires a Unix host")) }
}

/// A monotonic deadline survives the in-place exec into systemd-run and the
/// second helper phase, so the probe does not restart the 20-second allowance.
#[cfg(unix)]
pub fn latch_deadline_ns() -> u64 { unix::monotonic_ns().saturating_add(20_000_000_000) }
#[cfg(not(unix))]
pub fn latch_deadline_ns() -> u64 { 0 }

pub async fn wait_latch(receipt: &AgentLaunchReceipt, deadline_ns: u64, scope_entered: bool) -> Result<BTreeMap<String, String>, InspectionError> {
    #[cfg(unix)]
    { unix::wait(receipt, deadline_ns, scope_entered).await }
    #[cfg(not(unix))]
    { let _ = (receipt, deadline_ns, scope_entered); Err(failure("latch_unsupported", "private FIFO placement requires a Unix host")) }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{fs::File, io::{Read, Write}, os::fd::OwnedFd, path::Component};
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
    const DIRECTORY: OFlags = OFlags::RDONLY.union(OFlags::DIRECTORY).union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);
    fn uid() -> u32 { nix::unistd::geteuid().as_raw() }
    fn invalid(message: impl Into<String>) -> InspectionError { failure("latch_identity_invalid", message) }
    fn device(meta: &fs::Stat) -> u64 {
        #[cfg(target_vendor = "apple")]
        { u64::from(meta.st_dev as u32) }
        #[cfg(not(target_vendor = "apple"))]
        { meta.st_dev }
    }

    pub(super) fn monotonic_ns() -> u64 {
        let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        (now.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(now.tv_nsec as u64)
    }

    fn absolute_directory(path: &Path) -> Result<OwnedFd, InspectionError> {
        if !path.is_absolute() { return Err(invalid("latch directory must be absolute")); }
        let mut dir = fs::open("/", DIRECTORY, Mode::empty()).map_err(|e| invalid(e.to_string()))?;
        for part in path.components() {
            match part {
                Component::RootDir => {},
                Component::Normal(name) => { dir = fs::openat(&dir, name, DIRECTORY, Mode::empty()).map_err(|e| invalid(e.to_string()))?; },
                _ => return Err(invalid("latch directory traversal is forbidden")),
            }
        }
        Ok(dir)
    }

    fn metadata(fd: &OwnedFd) -> Result<fs::Stat, InspectionError> {
        fs::fstat(fd).map_err(|e| invalid(e.to_string()))
    }

    fn private_directory(parent: &OwnedFd, name: &str) -> Result<OwnedFd, InspectionError> {
        match fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) => {}, Err(rustix::io::Errno::EXIST) => {}, Err(e) => return Err(invalid(e.to_string())),
        }
        let fd = fs::openat(parent, name, DIRECTORY, Mode::empty()).map_err(|e| invalid(e.to_string()))?;
        let meta = metadata(&fd)?;
        if meta.st_uid != uid() || meta.st_mode & 0o7777 != 0o700 { return Err(invalid("latch directory must be UID-owned mode 0700")); }
        Ok(fd)
    }

    #[cfg(not(target_vendor = "apple"))]
    fn create_fifo(parent: &OwnedFd, _path: &Path, name: &str) -> Result<(), InspectionError> {
        fs::mknodat(parent, name, FileType::Fifo, Mode::from_raw_mode(0o600), 0)
            .map_err(|e| invalid(e.to_string()))
    }

    #[cfg(target_vendor = "apple")]
    fn create_fifo(parent: &OwnedFd, path: &Path, name: &str) -> Result<(), InspectionError> {
        // Apple has no mkfifoat. Its path-based operation is confined to this
        // private UID-owned directory, and the no-follow parent identity is
        // checked against the held descriptor before and after creation.
        // Deliberate same-UID namespace manipulation is not an OS sandbox.
        let held = metadata(parent)?;
        let check_parent = || {
            let actual = absolute_directory(path)?;
            let actual = metadata(&actual)?;
            if actual.st_dev != held.st_dev || actual.st_ino != held.st_ino
                || actual.st_uid != uid() || actual.st_mode & 0o7777 != 0o700 {
                return Err(invalid("private FIFO parent identity changed"));
            }
            Ok(())
        };
        check_parent()?;
        nix::unistd::mkfifo(&path.join(name), nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR)
            .map_err(|e| invalid(e.to_string()))?;
        check_parent()
    }

    pub(super) fn create(base: &Path, run_id: &str, epoch: u64) -> Result<AgentLaunchReceipt, InspectionError> {
        let base = if base.is_absolute() { base.to_owned() } else {
            std::env::current_dir().map_err(|e| invalid(e.to_string()))?.join(base)
        };
        if base.to_str().is_none() { return Err(invalid("state root cannot be represented in the literal CLI argv")); }
        let base_fd = absolute_directory(&base)?;
        let base_meta = metadata(&base_fd)?;
        if base_meta.st_uid != uid() || base_meta.st_mode & 0o022 != 0 { return Err(invalid("state root must be UID-owned and not writable by other users")); }
        // The store may already own an ordinary non-private orchestration
        // directory. Never chmod it: only latches itself is required 0700.
        match fs::mkdirat(&base_fd, "orchestration", Mode::from_raw_mode(0o700)) {
            Ok(()) => {}, Err(rustix::io::Errno::EXIST) => {}, Err(e) => return Err(invalid(e.to_string())),
        }
        let orchestration = fs::openat(&base_fd, "orchestration", DIRECTORY, Mode::empty()).map_err(|e| invalid(e.to_string()))?;
        let meta = metadata(&orchestration)?;
        if meta.st_uid != uid() || meta.st_mode & 0o022 != 0 { return Err(invalid("orchestration directory has unsafe ownership or permissions")); }
        let latches = private_directory(&orchestration, "latches")?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let digest = Sha256::digest(run_id.as_bytes());
        let run = u32::from_be_bytes(digest[..4].try_into().expect("SHA256 has four bytes"));
        let name = format!("{run:08x}-e{epoch}-{nonce}.fifo");
        let latch_directory = base.join("orchestration/latches");
        create_fifo(&latches, &latch_directory, &name)?;
        let fd = fs::openat(&latches, &name, OFlags::RDWR | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())
            .map_err(|e| invalid(e.to_string()))?;
        let meta = metadata(&fd)?;
        if !FileType::from_raw_mode(meta.st_mode).is_fifo() || meta.st_uid != uid() || meta.st_mode & 0o7777 != 0o600 || meta.st_nlink != 1 {
            let _ = fs::unlinkat(&latches, &name, AtFlags::empty());
            return Err(invalid("created FIFO permissions or ownership are invalid"));
        }
        Ok(AgentLaunchReceipt { latch: latch_directory.join(name).to_string_lossy().into_owned(), nonce,
            latch_device: device(&meta).to_string(), latch_inode: meta.st_ino.to_string() })
    }

    fn parent(receipt: &AgentLaunchReceipt) -> Result<(OwnedFd, &std::ffi::OsStr), InspectionError> {
        if receipt.nonce.len() != 32 || !receipt.nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || receipt.latch_device.parse::<u64>().is_err() || receipt.latch_inode.parse::<u64>().ok().filter(|i| *i > 0).is_none() {
            return Err(invalid("latch receipt is incomplete"));
        }
        let path = Path::new(&receipt.latch);
        let name = path.file_name().ok_or_else(|| invalid("missing latch file name"))?;
        if !name.to_str().is_some_and(|name| name.ends_with(&format!("-{}.fifo", receipt.nonce))) {
            return Err(invalid("latch name does not match its nonce"));
        }
        let path_parent = path.parent().ok_or_else(|| invalid("missing latch directory"))?;
        if path_parent.file_name() != Some(std::ffi::OsStr::new("latches"))
            || path_parent.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("orchestration")) {
            return Err(invalid("latch is outside an orchestration latches directory"));
        }
        let fd = absolute_directory(path_parent)?;
        let meta = metadata(&fd)?;
        if meta.st_uid != uid() || meta.st_mode & 0o7777 != 0o700 { return Err(invalid("latch parent is not private UID-owned 0700")); }
        Ok((fd, name))
    }

    fn exact(meta: &fs::Stat, receipt: &AgentLaunchReceipt) -> bool {
        FileType::from_raw_mode(meta.st_mode).is_fifo() && meta.st_uid == uid()
            && meta.st_mode & 0o7777 == 0o600 && meta.st_nlink == 1
            && receipt.latch_device.parse::<u64>() == Ok(device(meta)) && receipt.latch_inode.parse::<u64>() == Ok(meta.st_ino)
    }

    fn open(receipt: &AgentLaunchReceipt, flags: OFlags) -> Result<(OwnedFd, OwnedFd), InspectionError> {
        let (parent, name) = parent(receipt)?;
        let named = fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| invalid(e.to_string()))?;
        if !exact(&named, receipt) { return Err(invalid("named FIFO does not match its launch receipt")); }
        let fifo = match fs::openat(&parent, name, flags | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NXIO) => return Err(failure("latch_reader_absent", "waiting wrapper has not opened the exact FIFO")),
            Err(e) => return Err(invalid(e.to_string())),
        };
        if !exact(&metadata(&fifo)?, receipt) { return Err(invalid("FIFO identity differs from the durable launch receipt")); }
        Ok((parent, fifo))
    }

    fn startup_name(receipt: &AgentLaunchReceipt) -> Result<std::ffi::OsString, InspectionError> {
        let mut name = Path::new(&receipt.latch).file_name().ok_or_else(|| invalid("missing FIFO name"))?.to_os_string();
        name.push(".ready");
        Ok(name)
    }

    fn private_report(meta: &fs::Stat) -> bool {
        FileType::from_raw_mode(meta.st_mode).is_file() && meta.st_uid == uid()
            && meta.st_mode & 0o7777 == 0o600 && meta.st_nlink == 1
    }

    struct StartupMarker { parent: OwnedFd, name: std::ffi::OsString, device: u64, inode: u64 }
    impl Drop for StartupMarker {
        fn drop(&mut self) {
            if fs::statat(&self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|meta| private_report(&meta) && device(&meta) == self.device && meta.st_ino == self.inode) {
                let _ = fs::unlinkat(&self.parent, &self.name, AtFlags::empty());
            }
        }
    }

    fn publish_startup(receipt: &AgentLaunchReceipt, scope_entered: bool) -> Result<StartupMarker, InspectionError> {
        let (parent, fifo_name) = parent(receipt)?;
        let fifo = fs::statat(&parent, fifo_name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| invalid(e.to_string()))?;
        if !exact(&fifo, receipt) { return Err(invalid("FIFO changed before startup publication")); }
        let name = startup_name(receipt)?;
        let fd = fs::openat(&parent, &name, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600)).map_err(|e| invalid(e.to_string()))?;
        let meta = metadata(&fd)?;
        let marker = StartupMarker { parent, name, device: device(&meta), inode: meta.st_ino };
        if !private_report(&meta) { return Err(invalid("startup marker is not private UID-owned 0600")); }
        #[derive(Serialize)]
        struct BorrowedReport<'a> { nonce: &'a str, scope_entered: bool }
        let payload = serde_json::to_vec(&BorrowedReport { nonce: &receipt.nonce, scope_entered })
            .map_err(|e| invalid(e.to_string()))?;
        File::from(fd).write_all(&payload).map_err(|e| invalid(e.to_string()))?;
        // Publish completely before opening the FIFO reader: observing reader
        // readiness can never race a partially written startup report.
        Ok(marker)
    }

    pub(super) fn startup(receipt: &AgentLaunchReceipt) -> Result<Option<ScopeStartup>, InspectionError> {
        if !ready(receipt)? { return Ok(None); }
        let (parent, _) = parent(receipt)?;
        let name = startup_name(receipt)?;
        let fd = match fs::openat(&parent, &name, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(invalid(e.to_string())),
        };
        let meta = metadata(&fd)?;
        if !private_report(&meta) { return Err(invalid("startup report ownership or permissions differ")); }
        let mut bytes = Vec::with_capacity(513);
        File::from(fd).take(513).read_to_end(&mut bytes).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > 512 { return Err(invalid("startup report exceeds its bound")); }
        let report: StartupReport = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
        let fresh = fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| invalid(e.to_string()))?;
        if report.nonce != receipt.nonce || !private_report(&fresh)
            || fresh.st_dev != meta.st_dev || fresh.st_ino != meta.st_ino {
            return Err(invalid("startup report nonce or identity changed"));
        }
        Ok(Some(if report.scope_entered { ScopeStartup::Scoped } else { ScopeStartup::Uncontained }))
    }

    fn remove_exact(parent: &OwnedFd, receipt: &AgentLaunchReceipt) -> Result<(), InspectionError> {
        let name = Path::new(&receipt.latch).file_name().ok_or_else(|| invalid("missing FIFO name"))?;
        let fd = fs::openat(parent, name, OFlags::RDWR | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())
            .map_err(|e| invalid(e.to_string()))?;
        if !exact(&metadata(&fd)?, receipt) { return Err(invalid("FIFO was replaced before unlink")); }
        fs::unlinkat(parent, name, AtFlags::empty()).map_err(|e| invalid(e.to_string()))
    }

    pub(super) fn ready(receipt: &AgentLaunchReceipt) -> Result<bool, InspectionError> {
        match open(receipt, OFlags::WRONLY) {
            Ok(_) => Ok(true),
            Err(error) if error.code == "latch_reader_absent" => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn release(receipt: &AgentLaunchReceipt, frame: &[u8]) -> Result<(), InspectionError> {
        let (_, fd) = open(receipt, OFlags::WRONLY)?;
        let mut fifo = File::from(fd);
        match fifo.write(frame) {
            Ok(bytes) if bytes == frame.len() => Ok(()),
            Ok(_) => Err(failure("latch_release_unknown", "partial frame written; do not replay placement release")),
            Err(_) => Err(failure("latch_release_unknown", "placement write outcome is unknown; do not replay release")),
        }
    }

    pub(super) async fn wait(receipt: &AgentLaunchReceipt, deadline_ns: u64, scope_entered: bool) -> Result<BTreeMap<String, String>, InspectionError> {
        let _startup = publish_startup(receipt, scope_entered)?;
        // O_RDWR keeps our own write side open, avoiding immediate EOF/HUP and
        // repeated readiness wakes when the scheduler has not connected yet.
        let (parent, fd) = open(receipt, OFlags::RDWR)?;
        let allowance = deadline_ns.saturating_sub(monotonic_ns()).min(20_000_000_000);
        if allowance == 0 {
            let _ = remove_exact(&parent, receipt);
            return Err(failure("latch_timeout", "placement was not released before launch deadline"));
        }
        let fifo = tokio::io::unix::AsyncFd::new(File::from(fd)).map_err(|e| invalid(e.to_string()))?;
        let operation = async {
            let mut frame = vec![0; MAX_FRAME];
            let mut filled = 0;
            let mut expected = None;
            loop {
                let mut ready = fifo.readable().await.map_err(|e| invalid(e.to_string()))?;
                match ready.try_io(|inner| {
                    let mut file = inner.get_ref();
                    file.read(&mut frame[filled..])
                }) {
                    Err(_) => continue, // readiness is cleared by try_io
                    Ok(Err(e)) => return Err(invalid(e.to_string())),
                    Ok(Ok(0)) => return Err(failure("latch_payload_invalid", "unexpected placement FIFO EOF")),
                    Ok(Ok(bytes)) => filled += bytes,
                }
                if filled >= HEADER && expected.is_none() { expected = Some(frame_size(&frame[..filled])?); }
                if let Some(expected) = expected {
                    if filled >= expected { return decode(&frame[..filled], &receipt.nonce); }
                }
                if filled == MAX_FRAME { return Err(failure("latch_payload_invalid", "oversized placement frame")); }
            }
        };
        let result = tokio::time::timeout(Duration::from_nanos(allowance), operation).await
            .map_err(|_| failure("latch_timeout", "placement was not released before launch deadline"))
            .and_then(|result| result);
        // Only the exact private transient FIFO is removed, including on a
        // timeout/malformed frame. Never unlink a replaced caller path.
        let cleanup = remove_exact(&parent, receipt);
        match result { Ok(env) => { cleanup?; Ok(env) }, Err(error) => { let _ = cleanup; Err(error) } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receipt() -> AgentLaunchReceipt {
        AgentLaunchReceipt { latch: "/tmp/orchestration/latches/a-0123456789abcdef0123456789abcdef.fifo".into(),
            nonce: "0123456789abcdef0123456789abcdef".into(), latch_device: "1".into(), latch_inode: "2".into() }
    }
    #[test]
    fn frame_nonce_integrity_length_and_environment_are_exact() {
        let receipt = receipt();
        let env = BTreeMap::from([
            ("HERDR_PANE_ID".into(), "actual-pane".into()),
            ("COCKPIT_WORKSPACE_ID".into(), "actual-space".into()),
            ("COCKPIT_SOURCE_PATH".into(), "/original root/$literal".into()),
        ]);
        let frame = encode(&receipt, &env).unwrap();
        assert_eq!(decode(&frame, &receipt.nonce).unwrap(), env);
        assert!(decode(&frame, "wrong").is_err());
        for end in [0, HEADER - 1, HEADER, frame.len() - 1] { assert!(decode(&frame[..end], &receipt.nonce).is_err()); }
        let mut corrupt = frame.clone(); corrupt[HEADER] ^= 1;
        assert!(decode(&corrupt, &receipt.nonce).is_err());
        let mut extra = frame; extra.push(0);
        assert!(decode(&extra, &receipt.nonce).is_err());
        for key in ["PATH", "COCKPIT_JOB_CREDENTIAL_FILE", "OMP_API_KEY", "COCKPIT_HARNESS"] {
            assert!(encode(&receipt, &BTreeMap::from([(key.into(), "value".into())])).is_err());
        }
        assert!(encode(&receipt, &BTreeMap::from([("COCKPIT_CONTEXT_PATH".into(), "a\0b".into())])).is_err());
        assert!(encode(&receipt, &BTreeMap::from([("COCKPIT_CONTEXT_PATH".into(), "a".repeat(MAX_PAYLOAD))])).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn fifo_release_refuses_absent_reader_replacement_symlink_and_wrong_nonce() {
        let base = std::env::temp_dir().join(format!("cockpit-latch-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let launch = agent_launch(Path::new("/usr/bin/cockpit"), &base, "store", "run", &RunKind::Worker, 1, "label", ScopeMode::Off, vec!["harness".into()]).unwrap();
        assert_eq!(release_latch(&launch.receipt, &BTreeMap::new()).unwrap_err().code, "latch_reader_absent");
        let mut wrong = launch.receipt.clone(); wrong.nonce = "a".repeat(32);
        assert_eq!(release_latch(&wrong, &BTreeMap::new()).unwrap_err().code, "latch_identity_invalid");
        std::fs::remove_file(&launch.latch).unwrap();
        std::fs::write(&launch.latch, "not a FIFO").unwrap();
        assert_eq!(release_latch(&launch.receipt, &BTreeMap::new()).unwrap_err().code, "latch_identity_invalid");
        std::fs::remove_file(&launch.latch).unwrap();
        std::os::unix::fs::symlink("/dev/null", &launch.latch).unwrap();
        assert_eq!(release_latch(&launch.receipt, &BTreeMap::new()).unwrap_err().code, "latch_identity_invalid");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn complete_frame_releases_once_and_incomplete_frame_never_releases() {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let base = std::env::temp_dir().join(format!("cockpit-latch-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let env = BTreeMap::from([("COCKPIT_WORKSPACE_ID".into(), "placed".into())]);
        let launch = agent_launch(Path::new("/usr/bin/cockpit"), &base, "store", "run", &RunKind::Worker, 1, "label", ScopeMode::Off, vec!["harness".into()]).unwrap();
        assert!(!latch_ready(&launch.receipt).unwrap());
        assert_eq!(latch_startup(&launch.receipt).unwrap(), None);
        let receipt = launch.receipt.clone();
        let reader = tokio::spawn(async move { wait_latch(&receipt, latch_deadline_ns(), false).await });
        tokio::task::yield_now().await;
        assert!(latch_ready(&launch.receipt).unwrap());
        assert_eq!(latch_startup(&launch.receipt).unwrap(), Some(ScopeStartup::Uncontained));
        let report_path = PathBuf::from(format!("{}.ready", launch.receipt.latch));
        let report_bytes = std::fs::read(&report_path).unwrap();
        std::fs::write(&report_path, br#"{"nonce":"wrong","scope_entered":false}"#).unwrap();
        assert!(latch_startup(&launch.receipt).is_err());
        std::fs::write(&report_path, report_bytes).unwrap();
        assert_eq!(latch_startup(&launch.receipt).unwrap(), Some(ScopeStartup::Uncontained));
        release_latch(&launch.receipt, &env).unwrap();
        assert_eq!(reader.await.unwrap().unwrap(), env);
        assert!(!launch.latch.exists());
        assert!(!PathBuf::from(format!("{}.ready", launch.receipt.latch)).exists());
        assert!(release_latch(&launch.receipt, &env).is_err());

        let partial = agent_launch(Path::new("/usr/bin/cockpit"), &base, "store", "run", &RunKind::Worker, 2, "label", ScopeMode::Off, vec!["harness".into()]).unwrap();
        let receipt = partial.receipt.clone();
        let deadline = unix::monotonic_ns() + 500_000_000;
        let reader = tokio::spawn(async move { wait_latch(&receipt, deadline, false).await });
        tokio::task::yield_now().await;
        let mut writer = std::fs::OpenOptions::new().write(true).custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_NOFOLLOW).open(&partial.latch).unwrap();
        let frame = encode(&partial.receipt, &env).unwrap();
        assert_eq!(writer.write(&frame[..HEADER + 3]).unwrap(), HEADER + 3);
        drop(writer);
        assert_eq!(reader.await.unwrap().unwrap_err().code, "latch_timeout");
        assert!(!partial.latch.exists());
        assert!(!PathBuf::from(format!("{}.ready", partial.receipt.latch)).exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
