//! Bounded, non-polling invalidation for domain-authorized local reads.
//! Plans are crate-private: paths are supplied only by the owning services.
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, sync::{Arc, LazyLock, Mutex, Weak}, time::{Duration, Instant}};
use cockpit_protocol::local_changes::LocalChangeTopic;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;

const MAX_STREAMS: usize = 256;
const MAX_SOURCE_DIRECTORIES: usize = 4096;
const MAX_HOST_DIRECTORIES: usize = 8192;
const MAX_RULES: usize = 200_000;
const SETUP_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct LocalWatchError { pub code: &'static str, pub message: String }
impl LocalWatchError {
    fn new(code: &'static str, message: impl Into<String>) -> Self { Self { code, message: message.into() } }
}
impl std::fmt::Display for LocalWatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}: {}", self.code, self.message) }
}
impl std::error::Error for LocalWatchError {}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LocalWatchMatch { Exact, Children, MarkdownChildren, GitSharedIndexChildren, InvalidateExact, Lifecycle, LifecycleChildren, LifecycleDirectoryChildren }
#[derive(Clone, Debug)]
pub(crate) struct LocalWatchRule { pub path: PathBuf, pub match_kind: LocalWatchMatch, pub topics: Vec<LocalChangeTopic> }
pub(crate) struct LocalWatchPlan {
    directories: Vec<PathBuf>, rules: Vec<LocalWatchRule>, allowed_topics: Vec<LocalChangeTopic>,
    revalidate: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
}
impl LocalWatchPlan {
    pub(crate) fn new(directories: Vec<PathBuf>, rules: Vec<LocalWatchRule>, allowed_topics: Vec<LocalChangeTopic>, revalidate: Arc<dyn Fn() -> Result<(), String> + Send + Sync>) -> Self {
        Self { directories, rules, allowed_topics, revalidate }
    }
}
fn topic_index(topic: LocalChangeTopic) -> usize {
    match topic { LocalChangeTopic::Binding => 0, LocalChangeTopic::Scratchpad => 1, LocalChangeTopic::Todos => 2, LocalChangeTopic::Decisions => 3, LocalChangeTopic::Comments => 4, LocalChangeTopic::Listing => 5, LocalChangeTopic::FollowStatus => 6, LocalChangeTopic::Files => 7, LocalChangeTopic::Index => 8, LocalChangeTopic::SpaceGit => 9, LocalChangeTopic::SpaceTags => 10, LocalChangeTopic::Orchestration => 11 }
}
const TOPICS: [LocalChangeTopic; 12] = [LocalChangeTopic::Binding, LocalChangeTopic::Scratchpad, LocalChangeTopic::Todos, LocalChangeTopic::Decisions, LocalChangeTopic::Comments, LocalChangeTopic::Listing, LocalChangeTopic::FollowStatus, LocalChangeTopic::Files, LocalChangeTopic::Index, LocalChangeTopic::SpaceGit, LocalChangeTopic::SpaceTags, LocalChangeTopic::Orchestration];

/// A checkpoint records the mutation cursor before an event is sent, not after its ACK.
#[derive(Clone, Debug)]
pub struct LocalWatchSnapshot { pub cursor: u64, pub topics: Vec<LocalChangeTopic>, pub unavailable: Option<String>, revisions: [u64; 12] }
impl Default for LocalWatchSnapshot {
    fn default() -> Self { Self { cursor: 0, topics: Vec::new(), unavailable: None, revisions: [0; 12] } }
}
#[derive(Default)]
struct Counts { streams: usize }
fn counts() -> &'static Mutex<Counts> { static COUNTS: LazyLock<Mutex<Counts>> = LazyLock::new(|| Mutex::new(Counts::default())); &COUNTS }
struct Reservation { directories: usize }
impl Reservation {
    fn stream() -> Result<Self, LocalWatchError> {
        let mut counts = counts().lock().unwrap_or_else(|e| e.into_inner());
        if counts.streams >= MAX_STREAMS { return Err(LocalWatchError::new("watch_limit", "too many local change streams")); }
        counts.streams += 1; Ok(Self { directories: 0 })
    }
    fn add_directory(&mut self) -> Result<(), LocalWatchError> {
        if self.directories >= MAX_SOURCE_DIRECTORIES { return Err(LocalWatchError::new("watch_limit", "too many watched directories")); }
        self.directories += 1; Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) { counts().lock().unwrap_or_else(|e| e.into_inner()).streams -= 1; }
}

struct Filters { rules: Vec<LocalWatchRule>, allowed: [bool; 12] }
fn unavailable(state: &watch::Sender<LocalWatchSnapshot>, message: &'static str) {
    state.send_if_modified(|s| { if s.unavailable.is_some() { false } else { s.unavailable = Some(message.into()); true } });
}
fn structural(kind: &EventKind) -> bool { matches!(kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))) }
fn mutation(kind: &EventKind) -> bool { matches!(kind, EventKind::Any | EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other | EventKind::Access(notify::event::AccessKind::Close(notify::event::AccessMode::Write))) }
fn matches_rule(rule: &LocalWatchRule, path: &Path, kind: &EventKind) -> bool {
    let child = path.parent() == Some(rule.path.as_path());
    let markdown = path.extension().is_some_and(|ext| ext == "md");
    match rule.match_kind {
        LocalWatchMatch::Exact => path == rule.path,
        LocalWatchMatch::InvalidateExact => path == rule.path,
        LocalWatchMatch::Children => child,
        LocalWatchMatch::MarkdownChildren => child && markdown,
        LocalWatchMatch::GitSharedIndexChildren => child && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("sharedindex.") && !n.ends_with(".lock")),
        LocalWatchMatch::Lifecycle => structural(kind) && path == rule.path,
        LocalWatchMatch::LifecycleChildren => structural(kind) && child,
        LocalWatchMatch::LifecycleDirectoryChildren => child && matches!(kind,
            EventKind::Create(notify::event::CreateKind::Folder | notify::event::CreateKind::Any | notify::event::CreateKind::Other)
            | EventKind::Remove(notify::event::RemoveKind::Folder | notify::event::RemoveKind::Any | notify::event::RemoveKind::Other)
            | EventKind::Modify(notify::event::ModifyKind::Name(_))),
    }
}
#[cfg(test)]
fn callback(event: notify::Result<notify::Event>, filters: &Mutex<Filters>, state: &watch::Sender<LocalWatchSnapshot>) {
    match event { Ok(event) if !event.need_rescan() => callback_event(&event, filters, state), _ => unavailable(state, "filesystem observation failed or requires rescan") }
}
fn callback_event(event: &notify::Event, filters: &Mutex<Filters>, state: &watch::Sender<LocalWatchSnapshot>) {
    if !mutation(&event.kind) { return; }
    if event.paths.is_empty() { unavailable(state, "filesystem event could not be classified"); return; }
    let filters = filters.lock().unwrap_or_else(|e| e.into_inner());
    let mut topics = [false; 12];
    for rule in &filters.rules {
        if event.paths.iter().any(|p| matches_rule(rule, p, &event.kind)) {
            if matches!(rule.match_kind, LocalWatchMatch::InvalidateExact | LocalWatchMatch::Lifecycle | LocalWatchMatch::LifecycleChildren | LocalWatchMatch::LifecycleDirectoryChildren) { unavailable(state, "watched directory or configuration changed"); return; }
            for topic in &rule.topics { let i = topic_index(*topic); topics[i] |= filters.allowed[i]; }
        }
    }
    if topics.iter().any(|v| *v) {
        state.send_if_modified(|s| {
            if s.unavailable.is_some() { return false; }
            let Some(cursor) = s.cursor.checked_add(1) else { s.unavailable = Some("watch cursor exhausted".into()); return true; };
            s.cursor = cursor;
            for (i, dirty) in topics.iter().enumerate() { if *dirty { s.revisions[i] = cursor; } }
            true
        });
    }
}

struct Subscriber { filters: Mutex<Filters>, state: watch::Sender<LocalWatchSnapshot> }
type Routes = Arc<Mutex<HashMap<PathBuf, Vec<Weak<Subscriber>>>>>;
#[cfg(unix)]
type DirectoryIdentity = (u64, u64);
#[cfg(not(unix))]
type DirectoryIdentity = PathBuf;
struct Registry {
    watcher: Option<RecommendedWatcher>,
    identities: HashMap<PathBuf, DirectoryIdentity>,
    routes: Routes,
    failed: Arc<std::sync::atomic::AtomicBool>,
}
impl Default for Registry {
    fn default() -> Self { Self { watcher: None, identities: HashMap::new(), routes: Arc::new(Mutex::new(HashMap::new())), failed: Arc::new(std::sync::atomic::AtomicBool::new(false)) } }
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));
    &REGISTRY
}
struct DirectoryLeases { paths: HashSet<PathBuf>, subscriber: Arc<Subscriber> }
impl DirectoryLeases {
    fn register(&mut self, path: PathBuf, expected: &DirectoryIdentity) -> Result<(), LocalWatchError> {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        if registry.watcher.is_some() && registry.failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(LocalWatchError::new("watch_unavailable", "filesystem observer epoch failed"));
        }
        if let Some(identity) = registry.identities.get(&path) {
            if identity != expected { return Err(LocalWatchError::new("watch_identity", "shared watch root changed")); }
            registry.routes.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&path).expect("registered route").push(Arc::downgrade(&self.subscriber));
        } else {
            if registry.identities.len() >= MAX_HOST_DIRECTORIES { return Err(LocalWatchError::new("watch_limit", "too many watched directories")); }
            if registry.watcher.is_none() {
                registry.failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let failed = registry.failed.clone();
                let routes = registry.routes.clone();
                let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.as_ref().map_or(true, |event| event.need_rescan() || event.paths.is_empty() && mutation(&event.kind)) {
                        failed.store(true, std::sync::atomic::Ordering::Release);
                    }
                    let routes = routes.lock().unwrap_or_else(|e| e.into_inner());
                    for (directory, subscribers) in routes.iter() {
                        let relevant = match &event {
                            Ok(event) if !event.need_rescan() && !event.paths.is_empty() =>
                                event.paths.iter().any(|path| path == directory || path.parent() == Some(directory.as_path())),
                            _ => true,
                        };
                        if !relevant { continue; }
                        for subscriber in subscribers.iter().filter_map(Weak::upgrade) {
                            match &event {
                                Ok(event) if !event.need_rescan() => callback_event(event, &subscriber.filters, &subscriber.state),
                                _ => unavailable(&subscriber.state, "filesystem observation failed or requires rescan"),
                            }
                        }
                    }
                }).map_err(|e| LocalWatchError::new("watch_unavailable", e.to_string()))?;
                registry.watcher = Some(watcher);
            }
            registry.routes.lock().unwrap_or_else(|e| e.into_inner()).insert(path.clone(), vec![Arc::downgrade(&self.subscriber)]);
            let result = registry.watcher.as_mut().expect("watcher initialized").watch(&path, RecursiveMode::NonRecursive);
            if let Err(error) = result {
                registry.routes.lock().unwrap_or_else(|e| e.into_inner()).remove(&path);
                if registry.identities.is_empty() { registry.watcher.take(); }
                return Err(LocalWatchError::new("watch_unavailable", error.to_string()));
            }
            registry.identities.insert(path.clone(), expected.clone());
        }
        self.paths.insert(path);
        if registry.failed.load(std::sync::atomic::Ordering::Acquire) {
            unavailable(&self.subscriber.state, "filesystem observer epoch failed");
            return Err(LocalWatchError::new("watch_unavailable", "filesystem observer epoch failed"));
        }
        Ok(())
    }
}
impl Drop for DirectoryLeases {
    fn drop(&mut self) {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        for path in &self.paths {
            let remove = {
                let mut routes = registry.routes.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(subscribers) = routes.get_mut(path) {
                    subscribers.retain(|weak| weak.upgrade().is_some_and(|value| !Arc::ptr_eq(&value, &self.subscriber)));
                    if subscribers.is_empty() { routes.remove(path); true } else { false }
                } else { false }
            };
            if remove {
                registry.identities.remove(path);
                if registry.watcher.as_mut().is_some_and(|watcher| watcher.unwatch(path).is_err_and(|error| !matches!(error.kind, notify::ErrorKind::WatchNotFound))) {
                    let mut routes = registry.routes.lock().unwrap_or_else(|e| e.into_inner());
                    for subscribers in routes.values() {
                        for subscriber in subscribers.iter().filter_map(Weak::upgrade) { unavailable(&subscriber.state, "filesystem observer teardown failed"); }
                    }
                    routes.clear();
                    drop(routes);
                    registry.identities.clear();
                    registry.watcher.take();
                    break;
                }
            }
        }
        if registry.identities.is_empty() { registry.watcher.take(); }
    }
}

#[derive(Default, Clone)]
pub struct LocalChanges;
impl LocalChanges {
    pub(crate) async fn discovery_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        static PERMITS: LazyLock<Arc<tokio::sync::Semaphore>> = LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));
        PERMITS.clone().acquire_owned().await.expect("discovery semaphore is never closed")
    }
    pub(crate) fn subscribe(&self, plan: LocalWatchPlan) -> Result<LocalWatchLease, LocalWatchError> { self.begin(plan)?.finish() }
    pub(crate) fn begin(&self, mut plan: LocalWatchPlan) -> Result<LocalWatchBuilder, LocalWatchError> {
        if plan.rules.len() > MAX_RULES { return Err(LocalWatchError::new("watch_limit", "too many watch rules")); }
        (plan.revalidate)().map_err(|e| LocalWatchError::new("watch_identity", e))?;
        let reservation = Reservation::stream()?;
        let mut allowed = [false; 12]; for topic in &plan.allowed_topics { allowed[topic_index(*topic)] = true; }
        if plan.rules.iter().any(|r| r.topics.iter().any(|t| !allowed[topic_index(*t)])) { return Err(LocalWatchError::new("watch_plan", "rule has unauthorized topic")); }
        let (state, _) = watch::channel(LocalWatchSnapshot::default());
        let subscriber = Arc::new(Subscriber { filters: Mutex::new(Filters { rules: std::mem::take(&mut plan.rules), allowed }), state });
        let directories = DirectoryLeases { paths: HashSet::new(), subscriber };
        let mut builder = LocalWatchBuilder { directories, reservation, revalidate: plan.revalidate, started: Instant::now() };
        for directory in plan.directories { builder.watch_directory(directory, Vec::new())?; }
        Ok(builder)
    }
}

pub(crate) struct LocalWatchBuilder {
    directories: DirectoryLeases, reservation: Reservation,
    revalidate: Arc<dyn Fn() -> Result<(), String> + Send + Sync>, started: Instant,
}
impl LocalWatchBuilder {
    pub(crate) fn watch_directory(&mut self, path: PathBuf, rules: Vec<LocalWatchRule>) -> Result<(), LocalWatchError> {
        if self.started.elapsed() > SETUP_LIMIT { return Err(LocalWatchError::new("watch_limit", "watch setup timed out")); }
        if self.directories.paths.contains(&path) && rules.is_empty() { return Ok(()); }
        {
            let mut filters = self.directories.subscriber.filters.lock().unwrap_or_else(|e| e.into_inner());
            if filters.rules.len().saturating_add(rules.len()) > MAX_RULES || rules.iter().any(|r| r.topics.iter().any(|t| !filters.allowed[topic_index(*t)])) { return Err(LocalWatchError::new("watch_plan", "invalid or excessive watch rules")); }
            filters.rules.extend(rules);
        }
        if self.directories.paths.contains(&path) { return Ok(()); }
        let before = identity(&path)?;
        self.reservation.add_directory()?;
        self.directories.register(path.clone(), &before)?;
        if identity(&path)? != before { return Err(LocalWatchError::new("watch_identity", "directory changed while registering")); }
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<LocalWatchLease, LocalWatchError> {
        if self.started.elapsed() > SETUP_LIMIT { return Err(LocalWatchError::new("watch_limit", "watch setup timed out")); }
        (self.revalidate)().map_err(|e| LocalWatchError::new("watch_identity", e))?;
        if self.directories.subscriber.state.borrow().unavailable.is_some() { return Err(LocalWatchError::new("watch_unavailable", "watch changed during setup")); }
        Ok(LocalWatchLease { state: self.directories.subscriber.state.clone(), _directories: self.directories, _reservation: self.reservation, revalidate: self.revalidate })
    }
}
#[cfg(unix)]
fn identity(path: &Path) -> Result<(u64, u64), LocalWatchError> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path).map_err(|e| LocalWatchError::new("watch_unavailable", e.to_string()))?;
    if !m.is_dir() || m.file_type().is_symlink() { return Err(LocalWatchError::new("watch_identity", "watch target is not a directory")); }
    Ok((m.dev(), m.ino()))
}
#[cfg(not(unix))]
fn identity(path: &Path) -> Result<PathBuf, LocalWatchError> {
    let m = std::fs::symlink_metadata(path).map_err(|e| LocalWatchError::new("watch_unavailable", e.to_string()))?;
    if !m.is_dir() || m.file_type().is_symlink() { return Err(LocalWatchError::new("watch_identity", "watch target is not a directory")); }
    std::fs::canonicalize(path).map_err(|e| LocalWatchError::new("watch_unavailable", e.to_string()))
}

pub struct LocalWatchLease { _directories: DirectoryLeases, state: watch::Sender<LocalWatchSnapshot>, _reservation: Reservation, revalidate: Arc<dyn Fn() -> Result<(), String> + Send + Sync> }
impl LocalWatchLease {
    pub fn checkpoint(&self) -> LocalWatchSnapshot { self.state.borrow().clone() }
    pub fn receiver(&self) -> watch::Receiver<LocalWatchSnapshot> { self.state.subscribe() }
    pub fn revalidate(&self) -> Result<(), LocalWatchError> { (self.revalidate)().map_err(|e| LocalWatchError::new("watch_identity", e)) }
    pub fn take_since(&self, checkpoint: &LocalWatchSnapshot) -> LocalWatchSnapshot {
        let mut latest = self.checkpoint();
        latest.topics = TOPICS.into_iter().enumerate().filter_map(|(i, t)| (latest.revisions[i] > checkpoint.revisions[i]).then_some(t)).collect();
        latest
    }
}

/// One immutable domain authority and its observation lease.
pub struct LocalChangeSubscription {
    source: LocalChangeSource,
    authority: LocalAuthority,
}
enum LocalChangeSource {
    Filesystem(LocalWatchLease),
    Space { lease: crate::space_watch::SpaceWatchLease, topic: crate::space_watch::SpaceWatchTopic },
    Orchestration { lease: crate::orchestration::changes::OrchestrationChangeLease, _reservation: Reservation },
}
enum LocalChangeReceiverInner {
    Filesystem(watch::Receiver<LocalWatchSnapshot>),
    Space(watch::Receiver<crate::space_watch::SpaceWatchSnapshot>),
    Orchestration(watch::Receiver<crate::orchestration::changes::ChangeState>),
}
pub struct LocalChangeReceiver { inner: LocalChangeReceiverInner }
impl LocalChangeReceiver {
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        match &mut self.inner { LocalChangeReceiverInner::Filesystem(receiver) => receiver.changed().await, LocalChangeReceiverInner::Space(receiver) => receiver.changed().await, LocalChangeReceiverInner::Orchestration(receiver) => receiver.changed().await }
    }
}

enum LocalAuthority {
    Local,
    NotesBinding { service: Arc<crate::notes::NotesService>, session: String, space: String, endpoint: String },
    Viewer { service: Arc<crate::context::ContextService>, session: String, viewer: String, binding: String, root: String, pin: ViewerPin },
    LibraryIndex { service: Arc<crate::library::LibraryService>, root: String },
    Space { service: crate::CockpitService, session: String, server: String },
    Orchestration { service: crate::CockpitService, session: String, server: String, store: String, root: Option<String> },
}

#[derive(PartialEq, Eq)]
struct ViewerPin {
    server: String, endpoint: String, pane: String, terminal: String, workspace: String, tab: String,
    root_path: String, root_kind: cockpit_protocol::context::ContextRootKind, repository: String, checkout: String,
}
async fn viewer_pin(service: &crate::context::ContextService, session: &str, viewer: &str, binding: &str, root: &str) -> Result<ViewerPin, crate::InspectionError> {
    let authorization = service.authorize_viewer(session, viewer, binding).await?;
    let item = authorization.context.roots.iter().find(|item| item.root_id == root)
        .ok_or_else(|| crate::InspectionError::new("context_root_unavailable", "viewer root is unavailable"))?;
    let pin = ViewerPin {
        server: authorization.server_instance.clone(), endpoint: authorization.source.endpoint_identity.clone(),
        pane: authorization.source.pane_id.clone(), terminal: authorization.source.terminal_id.clone(),
        workspace: authorization.source.workspace_id.clone(), tab: authorization.source.tab_id.clone(),
        root_path: item.path.clone(), root_kind: item.kind, repository: item.repository_id.clone(), checkout: item.checkout_path.clone(),
    };
    service.authorize_files_root(session, viewer, binding, root).await?;
    Ok(pin)
}
impl LocalChangeSubscription {
    pub fn receiver(&self) -> LocalChangeReceiver {
        let inner = match &self.source {
            LocalChangeSource::Filesystem(lease) => LocalChangeReceiverInner::Filesystem(lease.receiver()),
            LocalChangeSource::Space { lease, .. } => LocalChangeReceiverInner::Space(lease.receiver()),
            LocalChangeSource::Orchestration { lease, .. } => LocalChangeReceiverInner::Orchestration(lease.receiver()),
        };
        LocalChangeReceiver { inner }
    }
    pub fn checkpoint(&self) -> LocalWatchSnapshot {
        match &self.source {
            LocalChangeSource::Filesystem(lease) => lease.checkpoint(),
            LocalChangeSource::Space { lease, topic } => {
                use crate::space_watch::{SpaceWatchStatus, SpaceWatchTopic};
                let checkpoint = lease.checkpoint();
                let (cursor, index, failed) = match topic {
                    SpaceWatchTopic::Git => (checkpoint.git, 9, checkpoint.git_initialized && checkpoint.git_watch == SpaceWatchStatus::Unavailable),
                    SpaceWatchTopic::Tags => (checkpoint.tags, 10, checkpoint.tags_watch == SpaceWatchStatus::Unavailable),
                };
                let mut snapshot = LocalWatchSnapshot::default();
                snapshot.cursor = cursor;
                snapshot.revisions[index] = cursor;
                if failed { snapshot.unavailable = Some("automatic updates unavailable".into()); }
                snapshot
            },
            LocalChangeSource::Orchestration { lease, .. } => {
                let state = lease.checkpoint();
                let mut snapshot = LocalWatchSnapshot::default();
                snapshot.cursor = state.generation;
                snapshot.revisions[11] = state.generation;
                snapshot.unavailable = state.unavailable;
                snapshot
            },
        }
    }
    pub fn take_since(&self, checkpoint: &LocalWatchSnapshot) -> LocalWatchSnapshot {
        let mut latest = self.checkpoint();
        latest.topics = TOPICS.into_iter().enumerate().filter_map(|(i, topic)| (latest.revisions[i] != checkpoint.revisions[i]).then_some(topic)).collect();
        latest
    }
    fn revalidate_filesystem(&self) -> Result<(), crate::InspectionError> {
        match &self.source {
            LocalChangeSource::Filesystem(lease) => lease.revalidate().map_err(|e| crate::InspectionError::new(e.code, e.message)),
            LocalChangeSource::Space { .. } | LocalChangeSource::Orchestration { .. } => Ok(()),
        }
    }
    /// Recheck the pinned authority before each delivery, outside the OS callback.
    pub async fn reauthorize(&self) -> Result<(), crate::InspectionError> {
        self.revalidate_filesystem()?;
        match &self.authority {
            LocalAuthority::Local => Ok(()),
            LocalAuthority::NotesBinding { service, session, space, endpoint } => service.reauthorize_binding_watch(session, space, endpoint).await,
            LocalAuthority::Viewer { service, session, viewer, binding, root, pin } => {
                if viewer_pin(service, session, viewer, binding, root).await? != *pin {
                    return Err(crate::InspectionError::new("context_identity_changed", "viewer authority changed"));
                }
                Ok(())
            },
            LocalAuthority::LibraryIndex { service, root } => { service.authorized_watch_root(root)?; Ok(()) },
            LocalAuthority::Space { service, session, server } => {
                if service.session_snapshot(session).await?.server_instance != *server {
                    return Err(crate::InspectionError::new("session_identity_mismatch", "session server incarnation changed"));
                }
                Ok(())
            },
            LocalAuthority::Orchestration { service, session, server, store, root } => {
                if service.session_snapshot(session).await?.server_instance != *server {
                    return Err(crate::InspectionError::new("session_identity_mismatch", "session server incarnation changed"));
                }
                service.orchestration()?.authorize_change_scope(session, store, root.as_deref())
            },
        }?;
        self.revalidate_filesystem()
    }
}

impl crate::CockpitService {
    pub async fn subscribe_local_changes(&self, scope: &cockpit_protocol::local_changes::LocalChangeScope) -> Result<LocalChangeSubscription, crate::InspectionError> {
        use cockpit_protocol::local_changes::LocalChangeScope;
        scope.validate().map_err(|e| crate::InspectionError::new("local_changes_invalid_scope", e))?;
        let changes = LocalChanges;
        if let LocalChangeScope::SpaceGit { session_id, server_instance } | LocalChangeScope::SpaceTags { session_id, server_instance } = scope {
            use crate::space_watch::SpaceWatchTopic;
            let topic = if matches!(scope, LocalChangeScope::SpaceGit { .. }) { SpaceWatchTopic::Git } else { SpaceWatchTopic::Tags };
            if self.session_snapshot(session_id).await?.server_instance != *server_instance {
                return Err(crate::InspectionError::new("session_identity_mismatch", "session server incarnation changed"));
            }
            let lease = self.space_watch_subscribe(session_id, server_instance, topic)?;
            let subscription = LocalChangeSubscription {
                source: LocalChangeSource::Space { lease, topic },
                authority: LocalAuthority::Space { service: self.clone(), session: session_id.clone(), server: server_instance.clone() },
            };
            subscription.reauthorize().await?;
            return Ok(subscription);
        }
        if let LocalChangeScope::Orchestration { session_id, server_instance, state_store_id, root_id } = scope {
            let reservation = Reservation::stream().map_err(|e| crate::InspectionError::new(e.code, e.message))?;
            let service = self.orchestration()?;
            let lease = service.change_lease(session_id, state_store_id, root_id.as_deref())?;
            let subscription = LocalChangeSubscription {
                source: LocalChangeSource::Orchestration { lease, _reservation: reservation },
                authority: LocalAuthority::Orchestration { service: self.clone(), session: session_id.clone(), server: server_instance.clone(), store: state_store_id.clone(), root: root_id.clone() },
            };
            subscription.reauthorize().await?;
            return Ok(subscription);
        }
        let (lease, authority) = match scope {
            LocalChangeScope::NotesContent { notes_id } => {
                let plan = self.notes()?.content_watch_plan(notes_id)?;
                (changes.subscribe(plan).map_err(|e| crate::InspectionError::new(e.code, e.message))?, LocalAuthority::Local)
            },
            LocalChangeScope::NotesBinding { session_id, space_id } => {
                let service = self.notes()?.clone();
                let (plan, endpoint) = service.binding_watch_plan(session_id, space_id).await?;
                let lease = changes.subscribe(plan).map_err(|e| crate::InspectionError::new(e.code, e.message))?;
                (lease, LocalAuthority::NotesBinding { service, session: session_id.clone(), space: space_id.clone(), endpoint })
            },
            LocalChangeScope::LibraryListing {} => {
                let plan = self.library()?.listing_watch_plan()?;
                (changes.subscribe(plan).map_err(|e| crate::InspectionError::new(e.code, e.message))?, LocalAuthority::Local)
            },
            LocalChangeScope::ContextFiles { session_id, viewer_id, binding_id, root_id, paths } => {
                let service = self.contexts()?.clone();
                let pin = viewer_pin(&service, session_id, viewer_id, binding_id, root_id).await?;
                let plan = service.watch_files_plan(session_id, viewer_id, binding_id, root_id, paths).await?;
                let lease = changes.subscribe(plan).map_err(|e| crate::InspectionError::new(e.code, e.message))?;
                (lease, LocalAuthority::Viewer { service, session: session_id.clone(), viewer: viewer_id.clone(), binding: binding_id.clone(), root: root_id.clone(), pin })
            },
            LocalChangeScope::ContextIndex { session_id, viewer_id, binding_id, root_id } => {
                let service = self.contexts()?.clone();
                let pin = viewer_pin(&service, session_id, viewer_id, binding_id, root_id).await?;
                let lease = service.watch_index(&changes, session_id, viewer_id, binding_id, root_id).await?;
                (lease, LocalAuthority::Viewer { service, session: session_id.clone(), viewer: viewer_id.clone(), binding: binding_id.clone(), root: root_id.clone(), pin })
            },
            LocalChangeScope::LibraryIndex { root_id } => {
                let service = self.library()?.clone();
                let lease = service.watch_index(&changes, root_id).await?;
                (lease, LocalAuthority::LibraryIndex { service, root: root_id.clone() })
            },
            LocalChangeScope::SpaceGit { .. } | LocalChangeScope::SpaceTags { .. } | LocalChangeScope::Orchestration { .. } => unreachable!("stream scopes handled above"),
        };
        let subscription = LocalChangeSubscription { source: LocalChangeSource::Filesystem(lease), authority };
        subscription.reauthorize().await?;
        Ok(subscription)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(path: &str) -> notify::Event { notify::Event::new(EventKind::Modify(notify::event::ModifyKind::Any)).add_path(path.into()) }
    #[test]
    fn directory_limit_refuses_excess_without_growing_reservation() {
        let mut reservation = Reservation::stream().unwrap();
        for _ in 0..MAX_SOURCE_DIRECTORIES { reservation.add_directory().unwrap(); }
        assert!(reservation.add_directory().is_err());
        assert_eq!(reservation.directories, MAX_SOURCE_DIRECTORIES);
    }
    #[test]
    fn coalesces_bursts_and_keeps_changes_across_checkpoints() {
        let (tx, _) = watch::channel(LocalWatchSnapshot::default());
        let filters = Mutex::new(Filters { rules: vec![LocalWatchRule { path: "/tmp/a".into(), match_kind: LocalWatchMatch::Exact, topics: vec![LocalChangeTopic::Files] }], allowed: [true; 12] });
        let before = tx.borrow().clone();
        for _ in 0..1_000_000 { callback(Ok(event("/tmp/a")), &filters, &tx); }
        let after = tx.borrow().clone();
        assert_eq!(after.cursor, 1_000_000); assert!(after.revisions[7] > before.revisions[7]);
        assert!(after.topics.is_empty());
        callback(Ok(event("/tmp/b")), &filters, &tx); assert_eq!(tx.borrow().cursor, after.cursor);
    }
    #[test]
    fn read_access_and_rescan_are_distinct() {
        let (tx, _) = watch::channel(LocalWatchSnapshot::default());
        let filters = Mutex::new(Filters { rules: vec![], allowed: [true; 12] });
        callback(Ok(notify::Event::new(EventKind::Access(notify::event::AccessKind::Read))), &filters, &tx);
        assert_eq!(tx.borrow().cursor, 0);
        callback(Ok(notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)), &filters, &tx);
        assert!(tx.borrow().unavailable.is_some());
    }
    #[test]
    fn concurrent_callbacks_preserve_latest_cursor() {
        let (state, _) = watch::channel(LocalWatchSnapshot::default());
        let filters = Arc::new(Mutex::new(Filters { rules: vec![LocalWatchRule { path: "/tmp/a".into(), match_kind: LocalWatchMatch::Exact, topics: vec![LocalChangeTopic::Files] }], allowed: [true; 12] }));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let state = state.clone();
                let filters = filters.clone();
                scope.spawn(move || for _ in 0..1000 { callback(Ok(event("/tmp/a")), &filters, &state); });
            }
        });
        let snapshot = state.borrow();
        assert_eq!(snapshot.cursor, 4000);
        assert_eq!(snapshot.revisions[7], snapshot.cursor);
    }
    #[test]
    fn invalidation_during_read_survives_old_checkpoint() {
        let (state, _) = watch::channel(LocalWatchSnapshot::default());
        let filters = Mutex::new(Filters { rules: vec![LocalWatchRule { path: "/tmp/a".into(), match_kind: LocalWatchMatch::Exact, topics: vec![LocalChangeTopic::Files] }], allowed: [true; 12] });
        let ready = state.borrow().clone();
        callback(Ok(event("/tmp/a")), &filters, &state);
        let after_read = state.borrow().clone();
        assert!(after_read.revisions[7] > ready.revisions[7]);
        callback(Ok(event("/tmp/a")), &filters, &state);
        assert!(state.borrow().revisions[7] > after_read.revisions[7]);
    }

    #[test]
    fn structural_change_invalidates_epoch_without_publishing_paths() {
        let (state, _) = watch::channel(LocalWatchSnapshot::default());
        let filters = Mutex::new(Filters { rules: vec![LocalWatchRule { path: "/tmp/root".into(), match_kind: LocalWatchMatch::LifecycleDirectoryChildren, topics: vec![] }], allowed: [true; 12] });
        callback(Ok(notify::Event::new(EventKind::Create(notify::event::CreateKind::Folder)).add_path("/tmp/root/new".into())), &filters, &state);
        let snapshot = state.borrow().clone();
        assert!(snapshot.unavailable.is_some());
        assert!(snapshot.topics.is_empty());
    }

    #[test]
    fn shared_directory_is_released_after_last_lease() {
        let root = std::env::temp_dir().join(format!("cockpit-shared-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let plan = || LocalWatchPlan::new(vec![root.clone()], Vec::new(), Vec::new(), Arc::new(|| Ok(())));
        let first = LocalChanges.subscribe(plan()).unwrap();
        let second = LocalChanges.subscribe(plan()).unwrap();
        {
            let entries = registry().lock().unwrap();
            assert_eq!(entries.routes.lock().unwrap().get(&root).unwrap().len(), 2);
        }
        drop(first);
        assert_eq!(registry().lock().unwrap().routes.lock().unwrap().get(&root).unwrap().len(), 1);
        drop(second);
        assert!(!registry().lock().unwrap().identities.contains_key(&root));
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn many_directories_share_one_observer() {
        let root = std::env::temp_dir().join(format!("cockpit-many-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let directories: Vec<_> = (0..160).map(|i| { let dir = root.join(i.to_string()); std::fs::create_dir(&dir).unwrap(); dir }).collect();
        let plan = LocalWatchPlan::new(directories.clone(), Vec::new(), Vec::new(), Arc::new(|| Ok(())));
        let lease = LocalChanges.subscribe(plan).unwrap();
        {
            let registry = registry().lock().unwrap();
            assert!(registry.watcher.is_some());
            assert!(directories.iter().all(|dir| registry.identities.contains_key(dir)));
        }
        drop(lease);
        for directory in directories { std::fs::remove_dir(directory).unwrap(); }
        std::fs::remove_dir(root).unwrap();
    }

    #[tokio::test]
    async fn registered_file_change_wakes_and_reads_do_not() {
        let root = std::env::temp_dir().join(format!("cockpit-local-watch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let file = root.join("test.md");
        std::fs::write(&file, "before").unwrap();
        let plan = LocalWatchPlan::new(vec![root.clone()], vec![LocalWatchRule {
            path: file.clone(), match_kind: LocalWatchMatch::Exact, topics: vec![LocalChangeTopic::Files],
        }], vec![LocalChangeTopic::Files], Arc::new(|| Ok(())));
        let lease = LocalChanges.subscribe(plan).unwrap();
        let mut receiver = lease.receiver();
        let ready = lease.checkpoint();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "before");
        std::fs::write(&file, "after").unwrap();
        tokio::time::timeout(Duration::from_secs(5), receiver.changed()).await.unwrap().unwrap();
        assert!(lease.take_since(&ready).topics.contains(&LocalChangeTopic::Files));
        drop(receiver);
        drop(lease);
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
