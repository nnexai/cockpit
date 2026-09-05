use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use crate::InspectionError;

const CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);

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
    mut command: Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
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
    loop {
        tokio::select! {
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

    let status = match tokio::time::timeout(
        deadline_at.saturating_duration_since(tokio::time::Instant::now()),
        child.child.as_mut().expect("owned child was lost").wait(),
    )
    .await
    {
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
    child.reaped = true;
    child.child.take();
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
