use std::{collections::{BTreeMap, BTreeSet}, path::{Path, PathBuf}, sync::{Arc, Mutex}};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;

use crate::InspectionError;

/// Cumulative filesystem causes. Each subscriber maintains its own cursor;
/// compare both fields with the values captured before starting a pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ChangeGenerations {
    pub(super) full: u64,
    pub(super) refs: u64,
}

fn publish(changes: &watch::Sender<u64>, classified: &watch::Sender<ChangeGenerations>,
    relevant: bool, causes: ChangeGenerations) {
    if relevant {
        changes.send_modify(|sequence| *sequence = sequence.wrapping_add(1));
    }
    if causes != ChangeGenerations::default() {
        classified.send_modify(|generation| {
            generation.full = generation.full.wrapping_add(causes.full);
            generation.refs = generation.refs.wrapping_add(causes.refs);
        });
    }
}

fn publish_failure(changes: &watch::Sender<u64>, classified: &watch::Sender<ChangeGenerations>,
    failure: &Mutex<Option<String>>, message: String) {
    *failure.lock().unwrap_or_else(|error| error.into_inner()) = Some(message);
    publish(changes, classified, true, ChangeGenerations { full: 1, refs: 0 });
}

/// Classify only mutations; a ref mixed with an unrecognized path is not
/// proof of a ref-only change. Git's lock source in an atomic rename is known.
pub(super) fn classify_event(event: &notify::Event, state: &Path, tasks: &Path,
    roots: &BTreeSet<PathBuf>, attempt_lifecycle: bool) -> ChangeGenerations {
    let mutation = matches!(event.kind, EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other)
        || matches!(event.kind, EventKind::Access(notify::event::AccessKind::Close(notify::event::AccessMode::Write)));
    if event.need_rescan() || matches!(event.kind, EventKind::Any | EventKind::Other) {
        return ChangeGenerations { full: 1, refs: 0 };
    }
    if !mutation && !attempt_lifecycle {
        return ChangeGenerations::default();
    }
    let mut full = attempt_lifecycle;
    let mut refs = false;
    let mut unknown = event.paths.is_empty();
    for path in &event.paths {
        if path == state || path == tasks
            || path.parent() == Some(tasks) && path.extension().is_some_and(|extension| extension == "md") {
            full = true;
            continue;
        }
        let mut known = false;
        for root in roots {
            if path == root {
                full = true;
                known = true;
                break;
            }
            if let Ok(relative) = path.strip_prefix(root) {
                if relative == Path::new("refs") {
                    full = true;
                    known = true;
                    break;
                }
                if relative == Path::new("HEAD") || relative == Path::new("packed-refs")
                    || relative.starts_with("refs") && !relative.extension().is_some_and(|extension| extension == "lock") {
                    if relative.starts_with("refs")
                        && matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))) {
                        match std::fs::symlink_metadata(path) {
                            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {},
                            _ => { full = true; known = true; break; }
                        }
                    }
                    refs = true;
                    known = true;
                    break;
                }
                if relative.extension().is_some_and(|extension| extension == "lock")
                    && (relative.starts_with("refs") || relative == Path::new("HEAD.lock") || relative == Path::new("packed-refs.lock")) {
                    known = true;
                    break;
                }
            }
        }
        unknown |= !known;
    }
    // Unrelated activity is filtered as before; ambiguity accompanying a
    // relevant event must not be downgraded to ref-only.
    full |= unknown && (refs || full || event.paths.is_empty());
    ChangeGenerations { full: u64::from(full), refs: u64::from(refs) }
}

#[cfg(test)]
pub(super) fn publish_test_event(changes: &watch::Sender<u64>, classified: &watch::Sender<ChangeGenerations>,
    event: &notify::Event, state: &Path, tasks: &Path, roots: &BTreeSet<PathBuf>, attempt_lifecycle: bool) {
    let causes = classify_event(event, state, tasks, roots, attempt_lifecycle);
    publish(changes, classified, causes != ChangeGenerations::default(), causes);
}

#[cfg(test)]
pub(super) fn publish_test_failure(changes: &watch::Sender<u64>,
    classified: &watch::Sender<ChangeGenerations>, failure: &Mutex<Option<String>>) {
    publish_failure(changes, classified, failure, "watch error".into());
}

/// Directory watches survive atomic state/task replacement. The receiver is
/// subscribed before reading durable state, so a concurrent commit stays queued.
pub(super) struct WaitNotifications {
    watcher: RecommendedWatcher,
    changes: watch::Sender<u64>,
    classified: watch::Sender<ChangeGenerations>,
    failure: Arc<Mutex<Option<String>>>,
    cleanup_roots: Arc<Mutex<BTreeSet<PathBuf>>>,
    cleanup_watches: BTreeMap<PathBuf, RecursiveMode>,
    attempts_path: PathBuf,
    attempt_dirs: Arc<Mutex<BTreeSet<PathBuf>>>,
    invalidated: Arc<Mutex<BTreeSet<PathBuf>>>,
}

impl WaitNotifications {
    pub(super) fn open(base: &Path) -> Result<Self, InspectionError> {
        let (changes, _) = watch::channel(0u64);
        let (classified, _) = watch::channel(ChangeGenerations::default());
        let classified_sender = classified.clone();
        let sender = changes.clone();
        let failure = Arc::new(Mutex::new(None));
        let callback_failure = Arc::clone(&failure);
        let cleanup_roots = Arc::new(Mutex::new(BTreeSet::<PathBuf>::new()));
        let callback_roots = Arc::clone(&cleanup_roots);
        let attempts_path = base.join("attempts");
        let watched_attempts = attempts_path.clone();
        let attempt_dirs = Arc::new(Mutex::new(BTreeSet::<PathBuf>::new()));
        let callback_attempt_dirs = Arc::clone(&attempt_dirs);
        let invalidated = Arc::new(Mutex::new(BTreeSet::<PathBuf>::new()));
        let callback_invalidated = Arc::clone(&invalidated);
        let state_path = base.join("state.json");
        let tasks_path = base.join("tasks");
        let watched_tasks = tasks_path.clone();
        let watched_base = base.to_owned();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let causes;
            let relevant = match event {
                Ok(event) => {
                    if matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)))
                        && event.paths.iter().any(|path| path == &watched_tasks || path == &watched_base) {
                        publish_failure(&sender, &classified_sender, &callback_failure,
                            "Watched orchestration directory was removed or renamed".into());
                        return;
                    }
                    if matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)))
                        && event.paths.iter().any(|path| callback_roots.lock().unwrap_or_else(|error| error.into_inner()).iter().any(|common| {
                            path == common || path.strip_prefix(common).is_ok_and(|relative| relative == Path::new("refs"))
                        })) {
                        publish_failure(&sender, &classified_sender, &callback_failure,
                            "Watched Git ref directory was removed or renamed".into());
                        return;
                    }
                    let entry_lifecycle = matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)));
                    let attempt_lifecycle = entry_lifecycle && event.paths.iter().any(|path| {
                        path == &watched_attempts || path.parent() == Some(watched_attempts.as_path())
                            || path.parent().is_some_and(|parent| callback_attempt_dirs.lock().unwrap_or_else(|error| error.into_inner()).contains(parent))
                    });
                    if matches!(event.kind, EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))) {
                        for path in &event.paths {
                            if path == &watched_attempts {
                                let dirs = callback_attempt_dirs.lock().unwrap_or_else(|error| error.into_inner());
                                let mut invalidated = callback_invalidated.lock().unwrap_or_else(|error| error.into_inner());
                                invalidated.insert(path.clone());
                                invalidated.extend(dirs.iter().cloned());
                            } else if callback_attempt_dirs.lock().unwrap_or_else(|error| error.into_inner()).contains(path) {
                                callback_invalidated.lock().unwrap_or_else(|error| error.into_inner()).insert(path.clone());
                            }
                        }
                    }
                    causes = classify_event(&event, &state_path, &watched_tasks,
                        &callback_roots.lock().unwrap_or_else(|error| error.into_inner()), attempt_lifecycle);
                    // Access/open/close-read events must not feed our own reads
                    // back into the wake channel. Include write-close events.
                    let mutation = matches!(event.kind, EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other)
                        || matches!(event.kind, EventKind::Access(notify::event::AccessKind::Close(notify::event::AccessMode::Write)));
                    event.need_rescan() || attempt_lifecycle || mutation && event.paths.iter().any(|path| {
                        path == &state_path || path == &watched_tasks
                            || path.parent() == Some(watched_tasks.as_path())
                                && path.extension().is_some_and(|extension| extension == "md")
                            || callback_roots.lock().unwrap_or_else(|error| error.into_inner()).iter().any(|common| {
                                path == common || path.strip_prefix(common).is_ok_and(|relative| {
                                    relative == Path::new("HEAD") || relative == Path::new("packed-refs") || relative.starts_with("refs")
                                        && !relative.extension().is_some_and(|extension| extension == "lock")
                                })
                            })
                    })
                }
                Err(error) => {
                    publish_failure(&sender, &classified_sender, &callback_failure, error.to_string());
                    return;
                }
            };
            publish(&sender, &classified_sender, relevant, causes);
        }).map_err(watch_error)?;
        watcher.watch(base, RecursiveMode::NonRecursive).map_err(watch_error)?;
        watcher.watch(&tasks_path, RecursiveMode::NonRecursive).map_err(watch_error)?;
        Ok(Self { watcher, changes, classified, failure, cleanup_roots, cleanup_watches: BTreeMap::new(),
            attempts_path, attempt_dirs, invalidated })
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(super) fn subscribe_classified(&self) -> watch::Receiver<ChangeGenerations> {
        self.classified.subscribe()
    }

    /// Refresh scoped Git refs and shallow attempt-output directory watches.
    /// Reads/content changes and object/index churn do not wake cleanup.
    pub(super) fn watch_cleanup_refs(&mut self, paths: &[PathBuf]) -> Result<(), InspectionError> {
        let roots: BTreeSet<_> = paths.iter().cloned().collect();
        let mut desired = BTreeMap::new();
        for common in &roots {
            desired.insert(common.clone(), RecursiveMode::NonRecursive);
            let refs = common.join("refs");
            match std::fs::symlink_metadata(&refs) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    desired.insert(refs, RecursiveMode::Recursive);
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Ok(_) => return Err(InspectionError::new("orchestration_watch", "Git refs watch path is not a directory")),
                Err(error) => return Err(InspectionError::new("orchestration_watch", error.to_string())),
            }
        }
        let mut attempt_dirs = BTreeSet::new();
        match std::fs::symlink_metadata(&self.attempts_path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                desired.insert(self.attempts_path.clone(), RecursiveMode::NonRecursive);
                for entry in std::fs::read_dir(&self.attempts_path).map_err(watch_io_error)? {
                    let entry = entry.map_err(watch_io_error)?;
                    let valid_id = entry.file_name().to_str().is_some_and(|name| uuid::Uuid::parse_str(name).is_ok());
                    if valid_id && entry.file_type().map_err(watch_io_error)?.is_dir() {
                        let path = entry.path();
                        desired.insert(path.clone(), RecursiveMode::NonRecursive);
                        attempt_dirs.insert(path);
                    }
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Ok(_) => return Err(InspectionError::new("orchestration_watch", "Attempt outputs watch path is not a directory")),
            Err(error) => return Err(watch_io_error(error)),
        }
        *self.attempt_dirs.lock().unwrap_or_else(|error| error.into_inner()) = attempt_dirs;
        let invalidated = std::mem::take(&mut *self.invalidated.lock().unwrap_or_else(|error| error.into_inner()));
        *self.cleanup_roots.lock().unwrap_or_else(|error| error.into_inner()) = roots;
        let obsolete: Vec<_> = self.cleanup_watches.keys().filter(|path| !desired.contains_key(*path) || invalidated.contains(*path)).cloned().collect();
        for path in obsolete {
            // Already removed directories no longer need an OS watch either.
            match self.watcher.unwatch(&path) {
                Ok(()) => {},
                Err(error) if matches!(error.kind, notify::ErrorKind::WatchNotFound) => {},
                Err(error) => return Err(watch_error(error)),
            }
            self.cleanup_watches.remove(&path);
        }
        for (path, mode) in desired {
            if !self.cleanup_watches.contains_key(&path) {
                self.watcher.watch(&path, mode).map_err(watch_error)?;
                self.cleanup_watches.insert(path, mode);
            }
        }
        Ok(())
    }

    pub(super) fn error(&self) -> Option<InspectionError> {
        self.failure.lock().unwrap_or_else(|error| error.into_inner()).as_ref()
            .map(|message| InspectionError::new("orchestration_watch", message.clone()))
    }
}

fn watch_error(error: notify::Error) -> InspectionError {
    InspectionError::new("orchestration_watch", error.to_string())
}

fn watch_io_error(error: std::io::Error) -> InspectionError {
    InspectionError::new("orchestration_watch", error.to_string())
}
