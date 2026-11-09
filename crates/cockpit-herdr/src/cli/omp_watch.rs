use std::{collections::BTreeSet, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicU8, Ordering}}, time::Duration};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{broadcast, mpsc, watch};

use super::{HerdrCliAdapter, SessionChange, agent_recap};

const INTERVAL: Duration = Duration::from_secs(5);

// Watching parent directories handles atomic replacement and journal rotation.
// Ancestors are watched as well so a removed/recreated directory is re-armed.
fn directories(paths: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    let mut dirs = BTreeSet::new();
    for path in paths {
        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir.is_dir() { dirs.insert(dir.to_owned()); }
            parent = dir.parent();
        }
    }
    dirs
}

fn relevant(event: &Event, targets: &BTreeSet<PathBuf>) -> bool {
    if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any | EventKind::Other) {
        return false;
    }
    event.paths.is_empty() || event.paths.iter().any(|path| {
        targets.iter().any(|target| path == target || target.starts_with(path))
    })
}

fn arm(targets: BTreeSet<PathBuf>, signals: watch::Sender<u64>, flags: Arc<AtomicU8>) -> notify::Result<RecommendedWatcher> {
    let targets = Arc::new(targets);
    let callback_targets = Arc::clone(&targets);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        match event {
            Ok(event) if relevant(&event, &callback_targets) => {
                if event.paths.is_empty() || event.paths.iter().any(|path| callback_targets.iter().any(|target| path != target && target.starts_with(path))) {
                    flags.fetch_or(1, Ordering::Relaxed);
                }
                signals.send_modify(|n| *n = n.wrapping_add(1));
            }
            Err(_) => {
                flags.fetch_or(2, Ordering::Relaxed);
                signals.send_modify(|n| *n = n.wrapping_add(1));
            }
            _ => {}
        }
    })?;
    let dirs = directories(&targets);
    if dirs.is_empty() {
        return Err(notify::Error::generic("no accessible OMP watch directory"));
    }
    for dir in dirs { watcher.watch(&dir, RecursiveMode::NonRecursive)?; }
    Ok(watcher)
}

fn targets(paths: &BTreeSet<PathBuf>, history: Option<&Path>) -> BTreeSet<PathBuf> {
    let mut targets = paths.clone();
    if !paths.is_empty() {
        if let Some(history) = history {
            targets.insert(history.to_owned());
            let mut wal = history.as_os_str().to_os_string();
            wal.push("-wal");
            targets.insert(PathBuf::from(wal));
        }
    }
    targets
}

pub(super) fn spawn(
    adapter: HerdrCliAdapter,
    session: String,
    sender: mpsc::Sender<SessionChange>,
    paths_rx: broadcast::Receiver<(String, BTreeSet<PathBuf>)>,
    has_omp: bool,
) {
    let refresh_session = session.clone();
    let closed = sender.clone();
    let mut skip_initial = !has_omp;
    tokio::spawn(async move {
        tokio::select! {
            _ = closed.closed() => {},
            _ = run(session, sender, paths_rx, agent_recap::database_path(), move || {
                let adapter = adapter.clone();
                let session = refresh_session.clone();
                let skip = std::mem::take(&mut skip_initial);
                async move { skip || adapter.read_snapshot(&session).await.is_ok() }
            }, arm) => {},
        }
    });
}

async fn run<F, Fut, A>(
    session: String,
    sender: mpsc::Sender<SessionChange>,
    mut paths_rx: broadcast::Receiver<(String, BTreeSet<PathBuf>)>,
    history: Option<PathBuf>,
    mut refresh: F,
    mut install: A,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
    A: FnMut(BTreeSet<PathBuf>, watch::Sender<u64>, Arc<AtomicU8>) -> notify::Result<RecommendedWatcher>,
{
        let (signals, mut events) = watch::channel(0u64);
        let flags = Arc::new(AtomicU8::new(0));
        let mut paths = BTreeSet::new();
        let mut watcher: Option<RecommendedWatcher> = None;
        let mut fallback;
        let mut pending = false;
        let mut next = tokio::time::Instant::now();
        // Subscribe before reading, then invalidate after arming: neither a write
        // during the read nor a write between the read and watch can be lost.
        fallback = !refresh().await;
        loop {
            let deadline = if fallback || pending { Some(next) } else { None };
            tokio::select! {
                _ = sender.closed() => break,
                update = paths_rx.recv() => match update {
                    Ok((id, new_paths)) if id == session && new_paths != paths => {
                        paths = new_paths;
                        watcher = None;
                        fallback = false;
                        if !paths.is_empty() {
                            match install(targets(&paths, history.as_deref()), signals.clone(), Arc::clone(&flags)) {
                                Ok(armed) => watcher = Some(armed),
                                Err(_) => fallback = true,
                            }
                            pending = true;
                            next = tokio::time::Instant::now();
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        fallback = true;
                        next = tokio::time::Instant::now();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    _ => {}
                },
                result = events.changed() => {
                    if result.is_err() { break; }
                    events.borrow_and_update();
                    let event_flags = flags.swap(0, Ordering::Relaxed);
                    if event_flags & 2 != 0 {
                        watcher = None;
                        fallback = true;
                    } else if event_flags & 1 != 0 && !paths.is_empty() {
                        // Re-arm on directory replacement, and cover the re-arm gap.
                        match install(targets(&paths, history.as_deref()), signals.clone(), Arc::clone(&flags)) {
                            Ok(armed) => { watcher = Some(armed); fallback = false; }
                            Err(_) => { watcher = None; fallback = true; }
                        }
                    }
                    pending = true;
                }
                _ = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => {
                    if fallback {
                        // Retry both discovery and watch installation at a bounded rate.
                        let discovered = refresh().await;
                        if discovered && paths.is_empty() {
                            fallback = false;
                        }
                        if discovered && !paths.is_empty() {
                            if let Ok(armed) = install(targets(&paths, history.as_deref()), signals.clone(), Arc::clone(&flags)) {
                                watcher = Some(armed);
                                fallback = false;
                            }
                        }
                    }
                    if (pending || fallback || watcher.is_some()) && sender.send(SessionChange::Changed).await.is_err() { break; }
                    pending = false;
                    next = tokio::time::Instant::now() + INTERVAL;
                }
            }
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[tokio::test]
    async fn actual_writes_rotation_and_reads() {
        let dir = std::env::temp_dir().join(format!("cockpit-omp-watch-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let journal = dir.join("session.jsonl");
        let wal = dir.join("history.db-wal");
        std::fs::write(&journal, b"initial\n").unwrap();
        let set = BTreeSet::from([journal.clone(), wal.clone()]);
        let (tx, mut rx) = watch::channel(0);
        let _watcher = arm(set, tx, Arc::new(AtomicU8::new(0))).unwrap();
        let mut data = String::new();
        std::fs::File::open(&journal).unwrap().read_to_string(&mut data).unwrap();
        std::fs::write(dir.join("unrelated"), b"x").unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(150), rx.changed()).await.is_err());
        writeln!(std::fs::OpenOptions::new().append(true).open(&journal).unwrap(), "next").unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.changed()).await.unwrap().unwrap();
        rx.borrow_and_update();
        std::fs::rename(&journal, dir.join("old")).unwrap();
        std::fs::write(&journal, b"new\n").unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.changed()).await.unwrap().unwrap();
        rx.borrow_and_update();
        std::fs::write(&wal, b"wal").unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.changed()).await.unwrap().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn subscription_switches_paths_and_stops_on_close() {
        let dir = std::env::temp_dir().join(format!("cockpit-omp-switch-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first");
        let second = dir.join("second");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        let (updates, paths) = broadcast::channel(8);
        let (sender, mut changes) = mpsc::channel(8);
        let task = tokio::spawn(run("test".into(), sender, paths, None, || async { true }, arm));
        updates.send(("test".into(), BTreeSet::from([first.clone()]))).unwrap();
        tokio::time::timeout(Duration::from_secs(2), changes.recv()).await.unwrap().unwrap();
        updates.send(("test".into(), BTreeSet::from([second.clone()]))).unwrap();
        tokio::time::timeout(Duration::from_secs(2), changes.recv()).await.unwrap().unwrap();
        std::fs::write(&first, b"obsolete").unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(200), changes.recv()).await.is_err());
        std::fs::write(&second, b"changed").unwrap();
        tokio::time::timeout(Duration::from_secs(6), changes.recv()).await.unwrap().unwrap();
        drop(changes);
        tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn failed_watch_falls_back_and_recovers() {
        use std::sync::atomic::AtomicUsize;
        let (updates, paths) = broadcast::channel(8);
        let (sender, mut changes) = mpsc::channel(8);
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let task = tokio::spawn(run("test".into(), sender, paths, None, || async { true },
            move |paths, signals, flags| {
                if count.fetch_add(1, Ordering::Relaxed) < 2 {
                    Err(notify::Error::generic("injected watch failure"))
                } else {
                    arm(paths, signals, flags)
                }
            }));
        updates.send(("test".into(), BTreeSet::from([std::env::temp_dir().join("omp-watch-recovery")]))).unwrap();
        changes.recv().await.unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        tokio::time::advance(INTERVAL).await;
        changes.recv().await.unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        tokio::time::advance(INTERVAL * 2).await;
        assert!(changes.try_recv().is_err(), "healthy watcher must stop fallback ticks");
        drop(changes);
        task.await.unwrap();
    }
    #[tokio::test(start_paused = true)]
    async fn failed_initial_discovery_retries_until_paths_are_watched() {
        use std::sync::atomic::AtomicUsize;
        let (updates, paths) = broadcast::channel(8);
        let (sender, mut changes) = mpsc::channel(8);
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let publisher = updates.clone();
        let journal = std::env::temp_dir().join("omp-discovery-recovery");
        let task = tokio::spawn(run("test".into(), sender, paths, None, move || {
            let count = Arc::clone(&count);
            let publisher = publisher.clone();
            let journal = journal.clone();
            async move {
                if count.fetch_add(1, Ordering::Relaxed) < 2 {
                    false
                } else {
                    publisher.send(("test".into(), BTreeSet::from([journal]))).unwrap();
                    true
                }
            }
        }, arm));
        changes.recv().await.unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        tokio::time::advance(INTERVAL).await;
        changes.recv().await.unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        tokio::time::advance(INTERVAL * 2).await;
        assert!(changes.try_recv().is_err());
        drop(changes);
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn lagged_empty_discovery_retries_after_failure() {
        use std::sync::atomic::AtomicUsize;
        let (updates, paths) = broadcast::channel(2);
        for _ in 0..4 {
            updates.send(("other".into(), BTreeSet::new())).unwrap();
        }
        let (sender, mut changes) = mpsc::channel(8);
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let publisher = updates.clone();
        let journal = std::env::temp_dir().join("omp-lag-recovery");
        let task = tokio::spawn(run("test".into(), sender, paths, None, move || {
            let count = Arc::clone(&count);
            let publisher = publisher.clone();
            let journal = journal.clone();
            async move {
                match count.fetch_add(1, Ordering::Relaxed) {
                    0 => true,
                    1 => false,
                    _ => {
                        publisher.send(("test".into(), BTreeSet::from([journal]))).unwrap();
                        true
                    }
                }
            }
        }, arm));
        changes.recv().await.unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        tokio::time::advance(INTERVAL).await;
        changes.recv().await.unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        drop(changes);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn identity_failure_stops_live_watcher_before_terminal_frame() {
        use super::super::{forward_subscription_aux, stop_subscription_aux};
        let dir = std::env::temp_dir().join(format!("cockpit-omp-identity-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let journal = dir.join("journal");
        std::fs::write(&journal, b"initial\n").unwrap();
        let (updates, paths) = broadcast::channel(8);
        let (aux_sender, aux_messages) = mpsc::channel(8);
        let (sender, mut messages) = mpsc::channel(8);
        let (lifetime, lifetime_rx) = watch::channel(());
        let mut lifetime = Some(lifetime);
        let mut forwarder = Some(tokio::spawn(forward_subscription_aux(aux_messages, sender.clone(), lifetime_rx)));
        let watcher = tokio::spawn(run("test".into(), aux_sender, paths, None, || async { true }, arm));
        updates.send(("test".into(), BTreeSet::from([journal.clone()]))).unwrap();
        assert!(matches!(tokio::time::timeout(Duration::from_secs(2), messages.recv()).await.unwrap(), Some(SessionChange::Changed)));
        stop_subscription_aux(&mut lifetime, &mut forwarder).await;
        sender.send(SessionChange::Disconnected {
            code: "session_identity_mismatch".into(), message: "identity changed".into(),
        }).await.unwrap();
        std::fs::write(&journal, b"after replacement\n").unwrap();
        drop(sender);
        assert!(matches!(messages.recv().await, Some(SessionChange::Disconnected { code, .. }) if code == "session_identity_mismatch"));
        assert!(tokio::time::timeout(Duration::from_secs(2), messages.recv()).await.unwrap().is_none());
        tokio::time::timeout(Duration::from_secs(2), watcher).await.unwrap().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
