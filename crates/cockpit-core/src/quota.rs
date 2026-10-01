//! Nonblocking, identity-free quota collection shared across host processes.
mod parse;
#[cfg(test)]
mod tests;

use crate::{
    config::QuotaConfiguration,
    process::run_bounded_command,
    project_store::{atomic_write_bytes, prepare_root, read_json_bounded},
};
use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::quota::*;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{process::Command, sync::Mutex};

const CADENCE: u64 = 300_000;
const STALE: u64 = 900_000;
const RETAIN: u64 = 86_400_000;
const MAX_CACHE: u64 = 65_536;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRecord {
    attempted_at_ms: Option<u64>,
    next_attempt_at_ms: u64,
    failures: u8,
    providers: Vec<ProviderRecord>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderRecord {
    provider: QuotaProvider,
    state: QuotaProviderState,
    error: Option<QuotaErrorCode>,
    accounts: Vec<QuotaAccount>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedQuota {
    schema: u32,
    omp: SourceRecord,
    gh: SourceRecord,
}
impl Default for PersistedQuota {
    fn default() -> Self {
        Self {
            schema: 1,
            omp: SourceRecord::default(),
            gh: SourceRecord::default(),
        }
    }
}
#[derive(Default)]
struct Memory {
    snapshot: PersistedQuota,
    collecting: bool,
    retry_at_ms: u64,
    cache_failures: u8,
    recheck: bool,
}

pub struct QuotaService {
    configuration: QuotaConfiguration,
    root: PathBuf,
    memory: Mutex<Memory>,
    #[cfg(test)]
    now: std::sync::atomic::AtomicU64,
}
impl QuotaService {
    /// Construction does no I/O and starts no subprocess.
    pub fn new(configuration: QuotaConfiguration, cache_root: &Path) -> Self {
        Self {
            configuration,
            root: cache_root.join("quota/v1"),
            memory: Mutex::new(Memory::default()),
            #[cfg(test)]
            now: std::sync::atomic::AtomicU64::new(0),
        }
    }
    fn now(&self) -> u64 {
        #[cfg(test)]
        {
            let now = self.now.load(std::sync::atomic::Ordering::Relaxed);
            if now != 0 {
                return now;
            }
        }
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
    /// Reads memory only; filesystem work, lock attempts and subprocesses run in the background.
    pub async fn status(self: &Arc<Self>) -> QuotaStatusResponse {
        let now = self.now();
        let mut memory = self.memory.lock().await;
        if !memory.collecting
            && now >= memory.retry_at_ms
            && (memory.recheck || due(&memory.snapshot.omp, now) || due(&memory.snapshot.gh, now))
        {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                memory.collecting = true;
                let service = Arc::clone(self);
                runtime.spawn(async move {
                    service.refresh().await;
                });
            }
        }
        response(&memory.snapshot, now, memory.collecting)
    }
    async fn refresh(self: Arc<Self>) {
        let root = self.root.clone();
        let now = self.now();
        let opened = tokio::task::spawn_blocking(move || open_cache(&root, now)).await;
        let (dir, lock, mut snapshot) = match opened {
            Ok(Ok(value)) => value,
            _ => {
                self.cache_failed().await;
                return;
            }
        };
        if lock.is_none() {
            let mut memory = self.memory.lock().await;
            if let Some(snapshot) = snapshot {
                memory.snapshot = snapshot;
            }
            memory.collecting = false;
            memory.retry_at_ms = now.saturating_add(3_000);
            memory.recheck = true;
            return;
        }
        let mut current = snapshot.take().unwrap_or_default();
        let omp_due = due(&current.omp, now);
        let gh_due = due(&current.gh, now);
        // Persist a lease before starting commands: a killed owner cannot cause a burst.
        for (record, is_due, providers) in [
            (
                &mut current.omp,
                omp_due,
                &[QuotaProvider::Codex, QuotaProvider::Claude][..],
            ),
            (&mut current.gh, gh_due, &[QuotaProvider::Copilot][..]),
        ] {
            if is_due {
                record.attempted_at_ms = Some(now);
                record.next_attempt_at_ms = now.saturating_add(CADENCE);
                if record.providers.is_empty() {
                    record.providers = providers
                        .iter()
                        .map(|provider| {
                            empty(
                                *provider,
                                QuotaProviderState::Unavailable,
                                Some(QuotaErrorCode::UsageUnavailable),
                            )
                        })
                        .collect();
                }
            }
        }
        if (omp_due || gh_due) && save(&dir, &current).is_err() {
            self.cache_failed().await;
            return;
        }
        {
            let mut memory = self.memory.lock().await;
            memory.snapshot = current.clone();
        }
        let omp = async {
            if omp_due {
                Some(self.collect(false).await)
            } else {
                None
            }
        };
        let gh = async {
            if gh_due {
                Some(self.collect(true).await)
            } else {
                None
            }
        };
        let (omp_result, gh_result) = tokio::join!(omp, gh);
        let finished = self.now();
        if let Some(result) = omp_result {
            apply(
                &mut current.omp,
                result,
                &[QuotaProvider::Codex, QuotaProvider::Claude],
                finished,
            );
        }
        if let Some(result) = gh_result {
            apply(&mut current.gh, result, &[QuotaProvider::Copilot], finished);
        }
        if save(&dir, &current).is_err() {
            let mut memory = self.memory.lock().await;
            mark_cache_failure(&mut memory.snapshot, finished);
            memory.collecting = false;
            memory.cache_failures = memory.cache_failures.saturating_add(1).min(5);
            memory.retry_at_ms = finished.saturating_add(backoff(memory.cache_failures));
            return;
        }
        let mut memory = self.memory.lock().await;
        memory.snapshot = current;
        memory.collecting = false;
        memory.retry_at_ms = 0;
        memory.cache_failures = 0;
        memory.recheck = false;
        // The advisory lock inode is deliberately never removed.
        drop(lock);
    }
    async fn cache_failed(&self) {
        let now = self.now();
        let mut memory = self.memory.lock().await;
        mark_cache_failure(&mut memory.snapshot, now);
        memory.collecting = false;
        memory.cache_failures = memory.cache_failures.saturating_add(1).min(5);
        memory.retry_at_ms = now.saturating_add(backoff(memory.cache_failures));
    }
    async fn collect(&self, gh: bool) -> Result<Vec<ProviderRecord>, QuotaErrorCode> {
        let executable = if gh {
            &self.configuration.gh_executable
        } else {
            &self.configuration.omp_executable
        };
        let mut command = Command::new(executable);
        command.current_dir(&self.root);
        if gh {
            command
                .args(["api", "--method", "GET", "copilot_internal/user"])
                .env("GH_PROMPT_DISABLED", "1")
                .env("GH_PAGER", "cat")
                .env("GH_HOST", "github.com")
                .env("GH_NO_UPDATE_NOTIFIER", "1")
                .env("NO_COLOR", "1");
        } else {
            command
                .args(["usage", "--json", "--redact", "--no-extensions"])
                .env("NO_COLOR", "1");
        }
        let output = run_bounded_command(
            command,
            2 * 1024 * 1024,
            8192,
            Duration::from_secs(if gh { 20 } else { 30 }),
            "quota source",
        )
        .await
        .map_err(|error| match error.code.as_str() {
            "execution_timeout" => QuotaErrorCode::Timeout,
            "execution_failed" => QuotaErrorCode::SourceMissing,
            "bounded_output" => QuotaErrorCode::Malformed,
            _ => QuotaErrorCode::Failed,
        })?;
        if !output.status.success() {
            // Classify locally; raw text is never logged, returned or persisted.
            let text = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
            if gh && text.contains("http 404") {
                return Ok(vec![empty(
                    QuotaProvider::Copilot,
                    QuotaProviderState::Unsupported,
                    Some(QuotaErrorCode::Unsupported),
                )]);
            }
            return Err(
                if gh
                    && (output.status.code() == Some(4)
                        || text.contains("not logged")
                        || text.contains("gh auth login")
                        || text.contains("http 401"))
                {
                    QuotaErrorCode::NotSignedIn
                } else {
                    QuotaErrorCode::Failed
                },
            );
        }
        if gh {
            parse::copilot(&output.stdout, self.now())
        } else {
            parse::omp(&output.stdout, self.now())
        }
    }
}
fn due(record: &SourceRecord, now: u64) -> bool {
    record.attempted_at_ms.is_none()
        || now >= record.next_attempt_at_ms
        || record
            .attempted_at_ms
            .is_some_and(|time| time > now.saturating_add(60_000))
}
fn open_cache(
    root: &Path,
    now: u64,
) -> Result<(Dir, Option<std::fs::File>, Option<PersistedQuota>), ()> {
    let (_, dir) = prepare_root(root, "quota").map_err(|_| ())?;
    #[cfg(unix)]
    {
        // cap-std directory handles can be O_PATH descriptors: open the pinned
        // directory itself before chmod rather than chmodding that handle.
        let permissions = dir.open(".").map_err(|_| ())?;
        rustix::fs::fchmod(&permissions, rustix::fs::Mode::from_raw_mode(0o700)).map_err(|_| ())?;
    }
    for name in ["collect.lock", "snapshot.json"] {
        match dir.symlink_metadata(name) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(()),
        }
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = dir
        .open_with("collect.lock", &options)
        .map_err(|_| ())?
        .into_std();
    if !file.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    #[cfg(unix)]
    rustix::fs::fchmod(&file, rustix::fs::Mode::from_raw_mode(0o600)).map_err(|_| ())?;
    let lock = match file.try_lock_exclusive() {
        Ok(()) => Some(file),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(_) => return Err(()),
    };
    let snapshot = read_json_bounded::<PersistedQuota>(&dir, "snapshot.json", MAX_CACHE)
        .ok()
        .filter(|snapshot| valid_snapshot(snapshot, now));
    Ok((dir, lock, snapshot))
}
fn save(dir: &Dir, snapshot: &PersistedQuota) -> Result<(), ()> {
    let bytes = serde_json::to_vec(snapshot).map_err(|_| ())?;
    if bytes.len() as u64 > MAX_CACHE {
        return Err(());
    }
    atomic_write_bytes(dir, "snapshot.json", &bytes).map_err(|_| ())
}
fn apply(
    record: &mut SourceRecord,
    result: Result<Vec<ProviderRecord>, QuotaErrorCode>,
    providers: &[QuotaProvider],
    now: u64,
) {
    record.attempted_at_ms = Some(now);
    match result {
        Ok(mut values) => {
            for value in &mut values {
                if value.state == QuotaProviderState::Unavailable
                    && value.error == Some(QuotaErrorCode::UsageUnavailable)
                {
                    if let Some(previous) = record.providers.iter().find(|previous| {
                        previous.provider == value.provider && !previous.accounts.is_empty()
                    }) {
                        value.accounts = previous.accounts.clone();
                        value.state = QuotaProviderState::Available;
                    }
                }
            }
            record.providers = values;
            record.failures = 0;
            record.next_attempt_at_ms = now.saturating_add(CADENCE);
        }
        Err(error) => {
            record.failures = record.failures.saturating_add(1).min(5);
            record.next_attempt_at_ms = now.saturating_add(backoff(record.failures));
            record.providers = providers
                .iter()
                .map(|provider| {
                    let mut value = record
                        .providers
                        .iter()
                        .find(|value| value.provider == *provider)
                        .cloned()
                        .unwrap_or_else(|| {
                            empty(*provider, QuotaProviderState::Unavailable, Some(error))
                        });
                    value.error = Some(error);
                    if value.accounts.is_empty() {
                        value.state = if error == QuotaErrorCode::NotSignedIn {
                            QuotaProviderState::NotSignedIn
                        } else {
                            QuotaProviderState::Unavailable
                        };
                    }
                    value
                })
                .collect();
        }
    }
}
fn backoff(failures: u8) -> u64 {
    (CADENCE << failures.saturating_sub(1)).min(3_600_000)
}
fn mark_cache_failure(snapshot: &mut PersistedQuota, now: u64) {
    for (record, providers) in [
        (
            &mut snapshot.omp,
            &[QuotaProvider::Codex, QuotaProvider::Claude][..],
        ),
        (&mut snapshot.gh, &[QuotaProvider::Copilot][..]),
    ] {
        apply(
            record,
            Err(QuotaErrorCode::CacheUnavailable),
            providers,
            now,
        );
    }
}
fn empty(
    provider: QuotaProvider,
    state: QuotaProviderState,
    error: Option<QuotaErrorCode>,
) -> ProviderRecord {
    ProviderRecord {
        provider,
        state,
        error,
        accounts: Vec::new(),
    }
}
fn response(snapshot: &PersistedQuota, now: u64, collecting: bool) -> QuotaStatusResponse {
    let providers = [
        QuotaProvider::Codex,
        QuotaProvider::Claude,
        QuotaProvider::Copilot,
    ]
    .into_iter()
    .map(|provider| {
        let source = if provider == QuotaProvider::Copilot {
            &snapshot.gh
        } else {
            &snapshot.omp
        };
        let mut value = source
            .providers
            .iter()
            .find(|value| value.provider == provider)
            .cloned()
            .unwrap_or_else(|| {
                empty(
                    provider,
                    if source.attempted_at_ms.is_none() {
                        QuotaProviderState::Pending
                    } else {
                        QuotaProviderState::Unavailable
                    },
                    source
                        .attempted_at_ms
                        .map(|_| QuotaErrorCode::UsageUnavailable),
                )
            });
        value.accounts.retain(|account| {
            account.fetched_at_ms <= now.saturating_add(60_000)
                && now.saturating_sub(account.fetched_at_ms) <= RETAIN
        });
        if value.state == QuotaProviderState::Available && value.accounts.is_empty() {
            value.state = QuotaProviderState::Unavailable;
            value.error = Some(value.error.unwrap_or(QuotaErrorCode::UsageUnavailable));
        }
        let fetched_at_ms = value
            .accounts
            .iter()
            .map(|account| account.fetched_at_ms)
            .min();
        QuotaProviderStatus {
            provider,
            state: value.state,
            error: value.error,
            fetched_at_ms,
            stale: fetched_at_ms
                .is_some_and(|time| value.error.is_some() || now.saturating_sub(time) > STALE),
            accounts: value.accounts,
        }
    })
    .collect();
    QuotaStatusResponse {
        generated_at_ms: now,
        collecting,
        providers,
    }
}
fn valid_snapshot(snapshot: &PersistedQuota, now: u64) -> bool {
    snapshot.schema == 1
        && valid_source(
            &snapshot.omp,
            &[QuotaProvider::Codex, QuotaProvider::Claude],
            now,
        )
        && valid_source(&snapshot.gh, &[QuotaProvider::Copilot], now)
}
fn valid_source(source: &SourceRecord, providers: &[QuotaProvider], now: u64) -> bool {
    if source.failures > 5 || source.next_attempt_at_ms > now.saturating_add(3_600_000 + 60_000) {
        return false;
    }
    if source.attempted_at_ms.is_none() {
        return source.providers.is_empty()
            && source.next_attempt_at_ms == 0
            && source.failures == 0;
    }
    source.providers.len() == providers.len()
        && source
            .providers
            .iter()
            .zip(providers)
            .all(|(value, provider)| {
                value.provider == *provider
                    && value.state != QuotaProviderState::Pending
                    && value.accounts.len() <= 8
                    && ((value.state == QuotaProviderState::Available)
                        == !value.accounts.is_empty())
                    && (value.state != QuotaProviderState::Unavailable || value.error.is_some())
                    && value.accounts.iter().all(|account| {
                        account.fetched_at_ms <= now.saturating_add(60_000)
                            && !account.limits.is_empty()
                            && account.limits.len() <= 24
                            && account
                                .limits
                                .iter()
                                .enumerate()
                                .all(|(index, limit)| parse::valid_limit(limit, *provider, index))
                    })
            })
}
