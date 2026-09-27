use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use crate::InspectionError;

const CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);

/// Aggregate allowance for one CLI download into a fresh private directory.
#[derive(Debug, Clone, Copy)]
pub struct StagingBudget {
    pub bytes: u64,
    pub max_files: usize,
}

const STAGING_CHECK_INTERVAL: Duration = Duration::from_millis(10);

fn staging_error() -> InspectionError {
    InspectionError::new(
        "source_attachment_size",
        "Attachment download exceeded its staging budget or created an unsafe entry",
    )
}

impl StagingBudget {
    fn check(self, dir: &cap_std::fs::Dir) -> Result<(), InspectionError> {
        let mut bytes = 0u64;
        // Never recurse or follow links. Stop at the first excess entry so the
        // work of each scan is bounded by the predicted match count.
        for (index, entry) in dir.entries().map_err(|_| staging_error())?.enumerate() {
            if index >= self.max_files {
                return Err(staging_error());
            }
            let entry = entry.map_err(|_| staging_error())?;
            let metadata = dir.symlink_metadata(entry.file_name()).map_err(|_| staging_error())?;
            if !metadata.is_file() {
                return Err(staging_error());
            }
            #[cfg(unix)]
            {
                use cap_std::fs::MetadataExt;
                if metadata.nlink() != 1 {
                    return Err(staging_error());
                }
            }
            bytes = bytes.checked_add(metadata.len()).ok_or_else(staging_error)?;
            if bytes > self.bytes {
                return Err(staging_error());
            }
        }
        Ok(())
    }
}

async fn watch_staging(staging: Option<(&cap_std::fs::Dir, StagingBudget)>) -> InspectionError {
    let Some((dir, budget)) = staging else {
        return std::future::pending().await;
    };
    let mut interval = tokio::time::interval(STAGING_CHECK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(error) = budget.check(dir) {
            return error;
        }
    }
}

async fn read_bounded_output<R: AsyncRead + Unpin>(
    reader: R,
    limit: usize,
    label: &str,
    stream: &str,
) -> Result<Vec<u8>, InspectionError> {
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    let mut reader = reader.take(limit.saturating_add(1) as u64);
    reader.read_to_end(&mut bytes).await.map_err(|_| {
        InspectionError::new(
            "bounded_output",
            format!("{label} {stream} could not be read"),
        )
    })?;
    if bytes.len() > limit {
        return Err(InspectionError::new(
            "bounded_output",
            format!("{label} {stream} exceeds configured limit"),
        ));
    }
    Ok(bytes)
}

pub struct OwnedChild {
    pub child: Option<Child>,
    pid: Option<u32>,
    pub reaped: bool,
}

impl OwnedChild {
    pub fn new(child: Child) -> Self {
        Self {
            pid: child.id(),
            child: Some(child),
            reaped: false,
        }
    }

    pub async fn kill_and_reap(&mut self) -> Option<String> {
        if self.reaped {
            return None;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            kill_process_group(pid);
        }
        let kill_error = self
            .child
            .as_mut()
            .and_then(|child| child.start_kill().err())
            .filter(|error| error.kind() != std::io::ErrorKind::NotFound)
            .map(|_| "child kill failed".to_owned());
        let wait_result = match self.child.as_mut() {
            Some(child) => Some(tokio::time::timeout(CHILD_CLEANUP_TIMEOUT, child.wait()).await),
            None => None,
        };
        let wait_error = match wait_result {
            Some(Ok(Ok(_))) => {
                self.child.take();
                self.reaped = true;
                None
            }
            Some(Ok(Err(_))) => {
                if let Some(child) = self.child.take() {
                    spawn_child_cleanup(child);
                }
                Some("child reap failed".to_owned())
            }
            Some(Err(_)) => {
                if let Some(child) = self.child.take() {
                    spawn_child_cleanup(child);
                }
                Some("child reap timed out".to_owned())
            }
            None => {
                self.reaped = true;
                None
            }
        };
        match (kill_error, wait_error) {
            (None, None) => None,
            (Some(error), None) | (None, Some(error)) => Some(error),
            (Some(kill), Some(wait)) => Some(format!("{kill}; {wait}")),
        }
    }
}

fn spawn_child_cleanup(mut child: Child) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let _ = tokio::time::timeout(CHILD_CLEANUP_TIMEOUT, child.wait()).await;
        });
    } else {
        drop(child);
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            kill_process_group(pid);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            spawn_child_cleanup(child);
        }
    }
}

#[cfg(unix)]
fn kill_process_group(pid: u32) {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
}

/// Run a finite subprocess while concurrently draining capped stdout and stderr.
///
/// The command is always owned by this future: timeout or dropped-future paths
/// terminate its process group and reap it within a bounded cleanup interval.
pub async fn run_bounded_command(
    command: Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
) -> Result<Output, InspectionError> {
    run_bounded_command_inner(command, stdout_limit, stderr_limit, timeout, label, None).await
}

/// Monitor a private flat download directory throughout execution, including
/// after stdout/stderr close. A breach kills/reaps the owned process group.
/// This is interval enforcement, not a filesystem quota: a write between scans
/// can overshoot the allowance. The caller owns and removes the staging tree.
pub async fn run_bounded_staging_command(
    command: Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
    dir: &cap_std::fs::Dir,
    budget: StagingBudget,
) -> Result<Output, InspectionError> {
    budget.check(dir)?;
    run_bounded_command_inner(command, stdout_limit, stderr_limit, timeout, label, Some((dir, budget))).await
}

async fn run_bounded_command_inner(
    mut command: Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
    staging: Option<(&cap_std::fs::Dir, StagingBudget)>,
) -> Result<Output, InspectionError> {
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            InspectionError::new("execution_failed", format!("{label} could not be started"))
        })?;
    let mut child = OwnedChild::new(child);
    let stdout = match child.child.as_mut().and_then(|child| child.stdout.take()) {
        Some(stdout) => stdout,
        None => {
            let cleanup = child.kill_and_reap().await;
            let message = cleanup.map_or_else(
                || format!("{label} stdout was not captured"),
                |cleanup| format!("{label} stdout was not captured; {cleanup}"),
            );
            return Err(InspectionError::new("execution_failed", message));
        }
    };
    let stderr = match child.child.as_mut().and_then(|child| child.stderr.take()) {
        Some(stderr) => stderr,
        None => {
            let cleanup = child.kill_and_reap().await;
            let message = cleanup.map_or_else(
                || format!("{label} stderr was not captured"),
                |cleanup| format!("{label} stderr was not captured; {cleanup}"),
            );
            return Err(InspectionError::new("execution_failed", message));
        }
    };

    let deadline_at = tokio::time::Instant::now() + timeout;
    let stdout_reader = read_bounded_output(stdout, stdout_limit, label, "stdout");
    let stderr_reader = read_bounded_output(stderr, stderr_limit, label, "stderr");
    tokio::pin!(stdout_reader);
    tokio::pin!(stderr_reader);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut stdout_result = None;
    let mut stderr_result = None;
    let monitor = watch_staging(staging);
    tokio::pin!(monitor);
    loop {
        tokio::select! {
            biased;
            error = &mut monitor => {
                let cleanup = child.kill_and_reap().await;
                return Err(with_cleanup(&error, cleanup));
            }
            result = &mut stdout_reader, if stdout_result.is_none() => stdout_result = Some(result),
            result = &mut stderr_reader, if stderr_result.is_none() => stderr_result = Some(result),
            _ = &mut deadline => {
                let cleanup = child.kill_and_reap().await;
                let message = cleanup.map_or_else(
                    || format!("{label} exceeded its deadline"),
                    |cleanup| format!("{label} exceeded its deadline; {cleanup}"),
                );
                return Err(InspectionError::new("execution_timeout", message));
            }
        }
        if let Some(Err(error)) = stdout_result.as_ref() {
            let cleanup = child.kill_and_reap().await;
            return Err(with_cleanup(error, cleanup));
        }
        if let Some(Err(error)) = stderr_result.as_ref() {
            let cleanup = child.kill_and_reap().await;
            return Err(with_cleanup(error, cleanup));
        }
        if stdout_result.is_some() && stderr_result.is_some() {
            break;
        }
    }

    let waited = tokio::select! {
        biased;
        error = &mut monitor => {
            let cleanup = child.kill_and_reap().await;
            return Err(with_cleanup(&error, cleanup));
        }
        result = tokio::time::timeout(
            deadline_at.saturating_duration_since(tokio::time::Instant::now()),
            child.child.as_mut().expect("owned child was lost").wait(),
        ) => result,
    };
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => {
            let cleanup = child.kill_and_reap().await;
            let message = cleanup.map_or_else(
                || format!("{label} did not finish"),
                |cleanup| format!("{label} did not finish; {cleanup}"),
            );
            return Err(InspectionError::new("execution_failed", message));
        }
        Err(_) => {
            let cleanup = child.kill_and_reap().await;
            let message = cleanup.map_or_else(
                || format!("{label} exceeded its deadline"),
                |cleanup| format!("{label} exceeded its deadline; {cleanup}"),
            );
            return Err(InspectionError::new("execution_timeout", message));
        }
    };
    // A download must not leave a descendant writing after the leader exits.
    #[cfg(unix)]
    if staging.is_some() {
        if let Some(pid) = child.pid {
            kill_process_group(pid);
        }
    }
    child.reaped = true;
    child.child.take();
    if let Some((dir, budget)) = staging {
        budget.check(dir)?;
    }
    Ok(Output {
        status,
        stdout: stdout_result
            .expect("stdout capture completed")
            .expect("stdout capture succeeded"),
        stderr: stderr_result
            .expect("stderr capture completed")
            .expect("stderr capture succeeded"),
    })
}

fn with_cleanup(error: &InspectionError, cleanup: Option<String>) -> InspectionError {
    match cleanup {
        Some(cleanup) => {
            InspectionError::new(error.code.clone(), format!("{}; {cleanup}", error.message))
        }
        None => error.clone(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    struct Staging {
        path: std::path::PathBuf,
        dir: cap_std::fs::Dir,
    }

    impl Staging {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-staging-budget-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            let dir = cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
            Self { path, dir }
        }
    }

    impl Drop for Staging {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn staging_budget_accepts_exact_boundary_and_rejects_aggregate_growth() {
        let staging = Staging::new();
        let budget = StagingBudget { bytes: 6, max_files: 2 };
        staging.dir.write("first", b"abc").unwrap();
        staging.dir.write("second", b"def").unwrap();
        budget.check(&staging.dir).unwrap();
        staging.dir.write("second", b"defg").unwrap();
        assert_eq!(budget.check(&staging.dir).unwrap_err().code, "source_attachment_size");
        staging.dir.write("second", b"def").unwrap();
        staging.dir.write("unexpected", b"").unwrap();
        assert_eq!(budget.check(&staging.dir).unwrap_err().code, "source_attachment_size");
    }

    #[test]
    fn staging_budget_rejects_symlinks_nested_directories_special_files_and_hardlinks() {
        for kind in ["symlink", "directory", "fifo", "hardlink"] {
            let staging = Staging::new();
            match kind {
                "symlink" => std::os::unix::fs::symlink("/does/not/exist", staging.path.join("unsafe")).unwrap(),
                "directory" => staging.dir.create_dir("unsafe").unwrap(),
                "fifo" => rustix::fs::mkfifoat(&staging.dir, "unsafe", rustix::fs::Mode::from_raw_mode(0o600)).unwrap(),
                "hardlink" => {
                    staging.dir.write("original", b"x").unwrap();
                    std::fs::hard_link(staging.path.join("original"), staging.path.join("unsafe")).unwrap();
                }
                _ => unreachable!(),
            }
            let error = StagingBudget { bytes: 100, max_files: 2 }.check(&staging.dir).unwrap_err();
            assert_eq!(error.code, "source_attachment_size", "{kind}");
        }
    }
}
