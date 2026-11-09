//! Filesystem invalidations for the sidebar's Git and tag reads.
//!
//! A newly armed Git watch advances its cursor after installation. A read that
//! raced installation is therefore always followed by another authoritative read.
use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Arc, Mutex}};
#[cfg(test)]
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpaceWatchStatus { Watching, Unavailable }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpaceWatchSnapshot {
    pub git: u64,
    pub tags: u64,
    pub git_watch: SpaceWatchStatus,
    pub tags_watch: SpaceWatchStatus,
}

#[cfg(test)]
pub(crate) struct TestWaitRequest {
    pub after_git: u64,
    pub after_tags: u64,
    pub timeout_ms: u32,
}

const MAX_SESSIONS: usize = 256;
const MAX_CURSOR: u64 = (1 << 53) - 1;
// Git follows local include/includeIf directives, whose targets may be outside
// the checkout. Do not claim complete observation for those configurations.
fn config_is_observable(path: &Path, required: bool) -> bool {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    let mut bytes = Vec::new();
    if file.take(1_048_577).read_to_end(&mut bytes).is_err() || bytes.len() > 1_048_576 {
        return false;
    }
    !bytes.windows(8).any(|window| window.eq_ignore_ascii_case(b"[include"))
}


fn bump(value: &mut u64) { *value = if *value >= MAX_CURSOR { 0 } else { *value + 1 }; }

#[derive(Clone, Copy)]
enum Topic { Git, Tags }

// A rescan means notifications were lost, including possibly the final write.
// Stop trusting this watcher and wake consumers so they enter fallback until
// an authoritative read has armed a fresh watch.
fn handle_rescan(
    event: &notify::Event,
    sender: &watch::Sender<SpaceWatchSnapshot>,
    failed: &std::sync::atomic::AtomicBool,
    topic: Topic,
) -> bool {
    if !event.need_rescan() { return false; }
    failed.store(true, std::sync::atomic::Ordering::Release);
    sender.send_if_modified(|state| {
        let (status, counter) = match topic {
            Topic::Git => (&mut state.git_watch, &mut state.git),
            Topic::Tags => (&mut state.tags_watch, &mut state.tags),
        };
        if *status == SpaceWatchStatus::Unavailable { false } else {
            *status = SpaceWatchStatus::Unavailable;
            bump(counter);
            true
        }
    });
    true
}

struct TopicWatch {
    _watcher: RecommendedWatcher,
    paths: Vec<(PathBuf, RecursiveMode)>,
}

struct Session {
    state: watch::Sender<SpaceWatchSnapshot>,
    git: Option<TopicWatch>,
    read_generation: u64,
    git_initialized: bool,
    tags: Option<TopicWatch>,
}

impl Session {
    fn new(tag_dir: Option<&Path>) -> Self {
        // A fresh cursor namespace distinguishes eviction or a host restart.
        let initial = ((uuid::Uuid::new_v4().as_u128() as u64) & MAX_CURSOR).max(1);
        let (state, _) = watch::channel(SpaceWatchSnapshot {
            git: initial, tags: initial, git_watch: SpaceWatchStatus::Unavailable, tags_watch: SpaceWatchStatus::Unavailable,
        });
        let mut session = Self { state, git: None, tags: None, read_generation: 0, git_initialized: false };
        session.arm_tags(tag_dir);
        session
    }

    fn arm_tags(&mut self, dir: Option<&Path>) {
        let Some(dir) = dir else { return; };
        let path = dir.to_owned();
        let targets = vec![(path.clone(), RecursiveMode::NonRecursive)];
        let sender = self.state.clone();
        let file = path.join("space-tags.json");
        let root = path.clone();
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_failed = Arc::clone(&failed);
        let result = create_watch(&targets, move |event| match event {
            Err(_) => {
                callback_failed.store(true, std::sync::atomic::Ordering::Release);
                sender.send_if_modified(|state| {
                    if state.tags_watch == SpaceWatchStatus::Unavailable { false } else {
                        state.tags_watch = SpaceWatchStatus::Unavailable; bump(&mut state.tags); true
                    }
                });
            }
            Ok(event) if handle_rescan(&event, &sender, &callback_failed, Topic::Tags) => {}
            Ok(event) if mutation(&event.kind) && !callback_failed.load(std::sync::atomic::Ordering::Acquire) => {
                if event.paths.iter().any(|p| p == &root) && matches!(event.kind, EventKind::Remove(_)) {
                    callback_failed.store(true, std::sync::atomic::Ordering::Release);
                    sender.send_modify(|state| { state.tags_watch = SpaceWatchStatus::Unavailable; bump(&mut state.tags); });
                } else if event.paths.iter().any(|p| p == &file) {
                    sender.send_modify(|state| bump(&mut state.tags));
                }
            }
            _ => {}
        });
        if let Ok(watcher) = result {
            self.tags = Some(TopicWatch { _watcher: watcher, paths: targets });
            self.state.send_modify(|state| {
                state.tags_watch = if failed.load(std::sync::atomic::Ordering::Acquire) { SpaceWatchStatus::Unavailable } else { SpaceWatchStatus::Watching };
                bump(&mut state.tags);
            });
        }
    }

    fn arm_git(&mut self, pairs: Vec<(PathBuf, PathBuf)>, complete: bool, tag_dir: Option<&Path>) {
        let tags_unavailable = matches!(self.state.borrow().tags_watch, SpaceWatchStatus::Unavailable);
        if tags_unavailable {
            self.tags = None;
            self.arm_tags(tag_dir);
        }
        let mut pairs: Vec<_> = pairs.into_iter().map(|(git, common)| {
            (std::fs::canonicalize(&git).unwrap_or(git), std::fs::canonicalize(&common).unwrap_or(common))
        }).collect();
        pairs.sort();
        pairs.dedup();
        let complete = complete && std::env::var_os("GIT_CONFIG_COUNT").is_none()
            && pairs.iter().all(|(git, common)| config_is_observable(&common.join("config"), true)
                && config_is_observable(&git.join("config.worktree"), false));
        let mut targets = Vec::new();
        for (git, common) in &pairs {
            targets.push((git.clone(), RecursiveMode::NonRecursive));
            targets.push((common.clone(), RecursiveMode::NonRecursive));
            targets.push((common.join("refs"), RecursiveMode::Recursive));
        }
        targets.sort_by(|a, b| a.0.cmp(&b.0));
        targets.dedup_by(|a, b| a.0 == b.0);
        if complete && matches!(self.state.borrow().git_watch, SpaceWatchStatus::Watching)
            && self.git.as_ref().is_some_and(|watch| watch.paths == targets) { return; }
        if !complete {
            self.git = None;
            self.state.send_if_modified(|state| {
                if matches!(state.git_watch, SpaceWatchStatus::Unavailable) { false } else {
                    state.git_watch = SpaceWatchStatus::Unavailable; bump(&mut state.git); true
                }
            });
            return;
        }
        let sender = self.state.clone();
        let watched = targets.clone();
        let observed: Vec<_> = pairs.into_iter().map(|(git, common)| (
            git.join("HEAD"), git.join("config.worktree"), common.join("config"),
            common.join("packed-refs"), common.join("shallow"), common.join("refs"),
        )).collect();
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_failed = Arc::clone(&failed);
        let result = create_watch(&targets, move |event| match event {
            Err(_) => {
                callback_failed.store(true, std::sync::atomic::Ordering::Release);
                sender.send_if_modified(|state| {
                    if state.git_watch == SpaceWatchStatus::Unavailable { false } else {
                        state.git_watch = SpaceWatchStatus::Unavailable; bump(&mut state.git); true
                    }
                });
            }
            Ok(event) if handle_rescan(&event, &sender, &callback_failed, Topic::Git) => {}
            Ok(event) if mutation(&event.kind) && !callback_failed.load(std::sync::atomic::Ordering::Acquire) => {
                let removed_watch = matches!(event.kind, EventKind::Remove(_)) && event.paths.iter().any(|p| watched.iter().any(|(root, _)| root == p));
                let relevant = event.paths.iter().any(|p| {
                    if p.file_name().is_some_and(|name| name.as_encoded_bytes().ends_with(b".lock")) { return false; }
                    observed.iter().any(|(head, worktree_config, config, packed, shallow, refs)|
                        p == head || p == worktree_config || p == config || p == packed || p == shallow || p.starts_with(refs))
                });
                if removed_watch {
                    callback_failed.store(true, std::sync::atomic::Ordering::Release);
                    sender.send_modify(|state| { state.git_watch = SpaceWatchStatus::Unavailable; bump(&mut state.git); });
                } else if relevant { sender.send_modify(|state| bump(&mut state.git)); }
            }
            _ => {}
        });
        match result {
            Ok(watcher) => {
                self.git = Some(TopicWatch { _watcher: watcher, paths: targets });
                self.state.send_modify(|state| {
                    state.git_watch = if failed.load(std::sync::atomic::Ordering::Acquire) { SpaceWatchStatus::Unavailable } else { SpaceWatchStatus::Watching };
                    bump(&mut state.git);
                });
            }
            Err(_) => {
                self.git = None;
                self.state.send_if_modified(|state| {
                    if matches!(state.git_watch, SpaceWatchStatus::Unavailable) { false } else {
                        state.git_watch = SpaceWatchStatus::Unavailable; bump(&mut state.git); true
                    }
                });
            }
        }
    }
}

fn mutation(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any | EventKind::Other)
}

fn create_watch(paths: &[(PathBuf, RecursiveMode)], callback: impl FnMut(notify::Result<notify::Event>) + Send + 'static) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(callback)?;
    for (path, mode) in paths { watcher.watch(path, *mode)?; }
    Ok(watcher)
}

#[derive(Default)]
pub(crate) struct SpaceWatch {
    sessions: Mutex<HashMap<(String, String), Arc<Mutex<Session>>>>,
    tag_dir: Mutex<Option<PathBuf>>,
    next_generation: std::sync::atomic::AtomicU64,
}

pub(crate) struct GitRead {
    session: Arc<Mutex<Session>>,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpaceWatchTopic { Git, Tags }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpaceWatchCheckpoint {
    pub git: u64,
    pub tags: u64,
    pub git_watch: SpaceWatchStatus,
    pub tags_watch: SpaceWatchStatus,
    pub git_initialized: bool,
}

pub(crate) struct SpaceWatchLease {
    session: Arc<Mutex<Session>>,
}

impl SpaceWatchLease {
    pub(crate) fn receiver(&self) -> watch::Receiver<SpaceWatchSnapshot> {
        self.session.lock().unwrap().state.subscribe()
    }

    pub(crate) fn checkpoint(&self) -> SpaceWatchCheckpoint {
        let session = self.session.lock().unwrap();
        let state = session.state.borrow();
        SpaceWatchCheckpoint {
            git: state.git, tags: state.tags, git_watch: state.git_watch,
            tags_watch: state.tags_watch, git_initialized: session.git_initialized,
        }
    }
}

impl SpaceWatch {
    pub(crate) fn set_tag_dir(&self, dir: &Path) { *self.tag_dir.lock().unwrap() = Some(dir.to_owned()); }

    fn session(&self, id: &str, server_instance: &str) -> Result<Arc<Mutex<Session>>, crate::InspectionError> {
        let mut sessions = self.sessions.lock().unwrap();
        let key = (id.to_owned(), server_instance.to_owned());
        if let Some(session) = sessions.get(&key) { return Ok(Arc::clone(session)); }
        // A subscription or in-flight read must retain the same entry that
        // subsequent status reads register against.
        if sessions.len() >= MAX_SESSIONS {
            let key = sessions.iter().find_map(|(key, session)|
                (Arc::strong_count(session) == 1).then(|| key.clone()))
                .ok_or_else(|| crate::InspectionError::new("watch_limit", "Too many active Space watches"))?;
            sessions.remove(&key);
        }
        let session = Arc::new(Mutex::new(Session::new(self.tag_dir.lock().unwrap().as_deref())));
        sessions.insert(key, Arc::clone(&session));
        Ok(session)
    }

    pub(crate) fn next_read(&self) -> u64 {
        self.next_generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed).wrapping_add(1)
    }

    pub(crate) fn begin_git_for(&self, id: &str, server_instance: &str, generation: u64) -> Result<GitRead, crate::InspectionError> {
        let session = self.session(id, server_instance)?;
        {
            let mut state = session.lock().unwrap();
            state.read_generation = state.read_generation.max(generation);
        }
        Ok(GitRead { session, generation })
    }

    #[cfg(test)]
    pub(crate) fn begin_git(&self, id: &str) -> Result<GitRead, crate::InspectionError> {
        self.begin_git_for(id, "test-server", self.next_read())
    }

    pub(crate) fn register_git(&self, read: GitRead, pairs: Vec<(PathBuf, PathBuf)>, complete: bool) {
        let tag_dir = self.tag_dir.lock().unwrap().clone();
        let mut session = read.session.lock().unwrap();
        if session.read_generation == read.generation {
            let first = !session.git_initialized;
            session.git_initialized = true;
            session.arm_git(pairs, complete, tag_dir.as_deref());
            if first {
                session.state.send_modify(|state| bump(&mut state.git));
            }
        }
    }

    pub(crate) fn subscribe(&self, id: &str, server_instance: &str, topic: SpaceWatchTopic) -> Result<SpaceWatchLease, crate::InspectionError> {
        let session = self.session(id, server_instance)?;
        if matches!(topic, SpaceWatchTopic::Tags) {
            let tag_dir = self.tag_dir.lock().unwrap().clone();
            let mut state = session.lock().unwrap();
            let unavailable = matches!(state.state.borrow().tags_watch, SpaceWatchStatus::Unavailable);
            if unavailable {
                state.tags = None;
                state.arm_tags(tag_dir.as_deref());
            }
        }
        Ok(SpaceWatchLease { session })
    }
    #[cfg(test)]
    pub(crate) async fn wait(&self, id: &str, request: &TestWaitRequest) -> SpaceWatchSnapshot {
        let session = match self.session(id, "test-server") {
            Ok(session) => session,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(u64::from(request.timeout_ms.min(30_000)))).await;
                return SpaceWatchSnapshot { git: 0, tags: 0, git_watch: SpaceWatchStatus::Unavailable, tags_watch: SpaceWatchStatus::Unavailable };
            }
        };
        let tag_dir = self.tag_dir.lock().unwrap().clone();
        let mut receiver = {
            let mut state = session.lock().unwrap();
            let tags_unavailable = matches!(state.state.borrow().tags_watch, SpaceWatchStatus::Unavailable);
            if tags_unavailable {
                state.tags = None;
                state.arm_tags(tag_dir.as_deref());
            }
            state.state.subscribe()
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(u64::from(request.timeout_ms.min(30_000)));
        loop {
            let current = receiver.borrow_and_update().clone();
            if current.git != request.after_git || current.tags != request.after_tags { return current; }
            if tokio::time::timeout_at(deadline, receiver.changed()).await.is_err() { return receiver.borrow().clone(); }
        }
    }
}

#[cfg(test)]
mod rescan_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn pathless_rescan_invalidates_each_topic_and_wakes_waiters() {
        for topic in [Topic::Git, Topic::Tags] {
            let (sender, receiver) = watch::channel(SpaceWatchSnapshot {
                git: 10, tags: 20,
                git_watch: SpaceWatchStatus::Watching,
                tags_watch: SpaceWatchStatus::Watching,
            });
            let failed = AtomicBool::new(false);
            let event = notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
            assert!(event.paths.is_empty());
            assert!(handle_rescan(&event, &sender, &failed, topic));
            assert!(failed.load(Ordering::Acquire));
            assert!(receiver.has_changed().unwrap());
            let result = receiver.borrow();
            match topic {
                Topic::Git => {
                    assert_eq!(result.git_watch, SpaceWatchStatus::Unavailable);
                    assert_eq!((result.git, result.tags), (11, 20));
                    assert_eq!(result.tags_watch, SpaceWatchStatus::Watching);
                }
                Topic::Tags => {
                    assert_eq!(result.tags_watch, SpaceWatchStatus::Unavailable);
                    assert_eq!((result.git, result.tags), (10, 21));
                    assert_eq!(result.git_watch, SpaceWatchStatus::Watching);
                }
            }
        }
    }
}
