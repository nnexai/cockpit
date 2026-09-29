use std::{
    collections::VecDeque,
    env,
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime},
};

use cap_std::fs::Dir;
use cockpit_protocol::context::{ContextFileIndexSource, ContextIndexedFile, ContextRootKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::project_store::{atomic_write_bytes, prepare_root, read_json_bounded};

const MAX_FILES: usize = 50_000;
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DISK_ENTRIES: usize = 32;
const MAX_MEMORY_ENTRIES: usize = 8;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const REWRITE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedIndex {
    schema: u32,
    kind: String,
    root: String,
    dev: u64,
    ino: u64,
    created_ms: u128,
    source: ContextFileIndexSource,
    truncated: bool,
    files: Vec<ContextIndexedFile>,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedIndex {
    pub files: Vec<ContextIndexedFile>,
    pub truncated: bool,
    pub source: ContextFileIndexSource,
}

struct MemoryEntry {
    key: String,
    value: CachedIndex,
    created: SystemTime,
    persisted: bool,
}

static MEMORY: LazyLock<Mutex<VecDeque<MemoryEntry>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

fn kind_name(kind: ContextRootKind) -> &'static str {
    match kind {
        ContextRootKind::Repository => "repository",
        ContextRootKind::Folder => "folder",
        ContextRootKind::Companion => "companion",
        ContextRootKind::Library => "library",
    }
}

fn source_name(source: ContextFileIndexSource) -> &'static str {
    match source {
        ContextFileIndexSource::Git => "git",
        ContextFileIndexSource::Walk => "walk",
    }
}

fn cache_path(configured: &Path) -> Option<PathBuf> {
    let path = env::var_os("COCKPIT_CACHE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| configured.to_path_buf());
    path.is_absolute().then(|| path.join("file-index").join("v1"))
}

fn root_identity(root: &Path) -> Option<(PathBuf, u64, u64)> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let canonical = std::fs::canonicalize(root).ok()?;
    let metadata = std::fs::metadata(&canonical).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        Some((canonical, metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        Some((canonical, 0, 0))
    }
}

fn cache_key(root: &Path, kind: ContextRootKind, dev: u64, ino: u64) -> Option<(String, PathBuf, String)> {
    let (canonical, _, _) = root_identity(root)?;
    let identity = format!("{}\0{}\0{}\0{}", kind_name(kind), canonical.to_string_lossy(), dev, ino);
    let digest = format!("{:x}", Sha256::digest(canonical.to_string_lossy().as_bytes()));
    let key = digest[..32].to_owned();
    Some((identity, canonical, key))
}

fn valid_files(files: &[ContextIndexedFile]) -> bool {
    files.len() <= MAX_FILES
        && files.iter().all(|file| {
            !file.path.is_empty()
                && file.path.len() <= 4096
                && !file.path.contains('\\')
                && !file.path.as_bytes().contains(&0)
                && file.path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
                && !Path::new(&file.path).is_absolute()
                && Path::new(&file.path).components().all(|component| matches!(component, Component::Normal(_)))
        })
        && files.windows(2).all(|pair| pair[0].path.as_bytes() < pair[1].path.as_bytes())
}

fn cache_dir(cache_root: &Path) -> Option<Dir> {
    let path = cache_path(cache_root)?;
    let (_, dir) = prepare_root(&path, "file index cache").ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        dir.open(".")
            .ok()?
            .into_std()
            .set_permissions(std::fs::Permissions::from_mode(0o700))
            .ok()?;
    }
    static STARTUP_SWEEP: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));
    let mut swept = STARTUP_SWEEP.lock().unwrap_or_else(|p| p.into_inner());
    if !*swept {
        let _ = sweep(&dir, "", 0);
        *swept = true;
    }
    Some(dir)
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn filename(key: &str) -> String {
    format!("{key}.json")
}

fn from_disk(
    root: &Path,
    kind: ContextRootKind,
    dev: u64,
    ino: u64,
    cache_root: &Path,
) -> Option<(CachedIndex, SystemTime)> {
    let (_identity, canonical, key) = cache_key(root, kind, dev, ino)?;
    let dir = cache_dir(cache_root)?;
    let name = filename(&key);
    let persisted: PersistedIndex = match read_json_bounded(&dir, &name, MAX_ENTRY_BYTES) {
        Ok(index) => index,
        Err(_) => {
            let _ = dir.remove_file(&name);
            return None;
        }
    };
    let age_ms = now_ms().saturating_sub(persisted.created_ms);
    if persisted.schema != SCHEMA
        || persisted.kind != source_name(persisted.source)
        || persisted.root != canonical.to_string_lossy()
        || persisted.dev != dev
        || persisted.ino != ino
        || age_ms > MAX_AGE.as_millis()
        || !valid_files(&persisted.files)
        || persisted.files.iter().any(|file| {
            let path = Path::new(&file.path);
            crate::context::reserved_context_path(kind, path).is_some()
                || (persisted.source == ContextFileIndexSource::Walk && crate::context_assets::excluded_source_path(path))
        })
    {
        let _ = dir.remove_file(&name);
        return None;
    }
    let created = SystemTime::UNIX_EPOCH + Duration::from_millis(persisted.created_ms.min(u64::MAX as u128) as u64);
    Some((CachedIndex { files: persisted.files, truncated: persisted.truncated, source: persisted.source }, created))
}

pub(crate) fn load(
    root: &Path,
    kind: ContextRootKind,
    cache_root: &Path,
) -> Option<CachedIndex> {
    let (canonical, dev, ino) = root_identity(root)?;
    let identity = format!("{}\0{}\0{}\0{}", kind_name(kind), canonical.to_string_lossy(), dev, ino);
    let now = SystemTime::now();
    {
        let mut entries = MEMORY.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(index) = entries.iter().position(|entry| entry.key == identity) {
            let entry = entries.remove(index)?;
            if now.duration_since(entry.created).unwrap_or_default() <= MAX_AGE {
                let result = entry.value.clone();
                entries.push_front(entry);
                return Some(result);
            }
        }
    }
    if kind == ContextRootKind::Library { return None; }
    let (value, created) = from_disk(root, kind, dev, ino, cache_root)?;
    remember(identity, value.clone(), created, true);
    Some(value)
}

fn remember(key: String, value: CachedIndex, created: SystemTime, persisted: bool) {
    let mut entries = MEMORY.lock().unwrap_or_else(|p| p.into_inner());
    entries.retain(|entry| entry.key != key);
    entries.push_front(MemoryEntry { key, value, created, persisted });
    entries.truncate(MAX_MEMORY_ENTRIES);
}

pub(crate) fn store(
    root: &Path,
    kind: ContextRootKind,
    source: ContextFileIndexSource,
    mut truncated: bool,
    mut files: Vec<ContextIndexedFile>,
    cache_root: &Path,
) {
    files.sort_unstable_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    files.dedup_by(|left, right| left.path == right.path);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        truncated = true;
    }
    if !valid_files(&files) { return; }
    let Some((dev, ino)) = root_identity(root).map(|(_, dev, ino)| (dev, ino)) else { return };
    let Some((identity, canonical, key)) = cache_key(root, kind, dev, ino) else { return };
    let now = SystemTime::now();
    let (previous_created, contents_changed) = {
        let entries = MEMORY.lock().unwrap_or_else(|p| p.into_inner());
        entries.iter().find(|entry| entry.key == identity).map_or((None, true), |entry| {
            (
                Some(entry.created),
                !entry.persisted
                    || now.duration_since(entry.created).unwrap_or_default() >= REWRITE_AFTER
                    || entry.value.truncated != truncated
                    || entry.value.source != source
                    || entry.value.files.len() != files.len()
                    || !entry.value.files.iter().zip(&files).all(|(cached, fresh)| cached.path == fresh.path),
            )
        })
    };
    let disk_dir = (kind != ContextRootKind::Library).then(|| cache_dir(cache_root)).flatten();
    let name = filename(&key);
    let disk_entry_exists = disk_dir.as_ref().is_some_and(|dir| {
        dir.symlink_metadata(&name)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    });
    let write_needed = contents_changed || (disk_dir.is_some() && !disk_entry_exists);
    for file in &mut files {
        file.bytes = None;
    }
    let value = CachedIndex { files: files.clone(), truncated, source };
    remember(identity.clone(), value, previous_created.unwrap_or(now), !write_needed && disk_entry_exists);
    if kind == ContextRootKind::Library || !write_needed { return; }
    let write_created = SystemTime::now();
    let persisted = PersistedIndex {
        schema: SCHEMA,
        kind: source_name(source).to_owned(),
        root: canonical.to_string_lossy().into_owned(),
        dev,
        ino,
        created_ms: write_created.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis(),
        source,
        truncated,
        files,
    };
    let Ok(bytes) = serde_json::to_vec(&persisted) else { return };
    if bytes.len() as u64 > MAX_ENTRY_BYTES { return; }
    let Some(dir) = disk_dir else { return };
    sweep(&dir, &name, bytes.len() as u64);
    match atomic_write_bytes(&dir, &name, &bytes) {
        Ok(()) => {
            let mut entries = MEMORY.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(entry) = entries.iter_mut().find(|entry| entry.key == identity) {
                entry.created = write_created;
                entry.persisted = true;
            }
        }
        Err(error) => eprintln!("file-index cache write failed: {error}"),
    }
}

fn sweep(dir: &Dir, target: &str, new_bytes: u64) {
    struct Entry { name: String, age_ms: u128, bytes: u64 }
    let Ok(entries) = dir.entries() else { return };
    let now = cap_std::time::SystemTime::from_std(SystemTime::now());
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") { continue; }
        let Ok(metadata) = dir.symlink_metadata(&name) else { continue };
        if metadata.file_type().is_symlink() || !metadata.is_file() { let _ = dir.remove_file(&name); continue; }
        let age_ms = metadata.modified().ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|duration| u128::from(duration.as_secs()) * 1000 + u128::from(duration.subsec_nanos()) / 1_000_000)
            .unwrap_or_default();
        if metadata.len() > MAX_ENTRY_BYTES || age_ms > MAX_AGE.as_millis() {
            let _ = dir.remove_file(&name);
            continue;
        }
        total = total.saturating_add(metadata.len());
        files.push(Entry { name, age_ms, bytes: metadata.len() });
    }
    files.sort_by(|left, right| right.age_ms.cmp(&left.age_ms));
    let target_existing = files.iter().find(|entry| entry.name == target).map_or(0, |entry| entry.bytes);
    total = total.saturating_sub(target_existing);
    let mut count_after_write = files.len() + usize::from(!target.is_empty() && target_existing == 0);
    while count_after_write > MAX_DISK_ENTRIES || total.saturating_add(new_bytes) > MAX_CACHE_BYTES {
        let Some(index) = files.iter().position(|entry| entry.name != target) else { break };
        let removed = files.remove(index);
        total = total.saturating_sub(removed.bytes);
        count_after_write = count_after_write.saturating_sub(1);
        let _ = dir.remove_file(&removed.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_round_trip_restores_sorted_path_only_entries() {
        if env::var_os("COCKPIT_CACHE_ROOT").is_some() {
            return;
        }
        let base = env::temp_dir().join(format!("cockpit-file-index-{}", uuid::Uuid::new_v4()));
        let root = base.join("checkout");
        let configured_cache = base.join("cache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&configured_cache).unwrap();
        let files = vec![
            ContextIndexedFile { path: "z.md".into(), bytes: Some(9) },
            ContextIndexedFile { path: "a.md".into(), bytes: Some(4) },
        ];
        store(
            &root,
            ContextRootKind::Folder,
            ContextFileIndexSource::Walk,
            false,
            files,
            &configured_cache,
        );
        let (canonical, dev, ino) = root_identity(&root).unwrap();
        let identity = format!(
            "{}\0{}\0{}\0{}",
            kind_name(ContextRootKind::Folder),
            canonical.to_string_lossy(),
            dev,
            ino
        );
        MEMORY.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|entry| entry.key != identity);
        let loaded = load(&root, ContextRootKind::Folder, &configured_cache).unwrap();
        assert_eq!(
            loaded.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
            vec!["a.md", "z.md"]
        );
        assert!(loaded.files.iter().all(|file| file.bytes.is_none()));
        assert_eq!(loaded.source, ContextFileIndexSource::Walk);

        let directory = configured_cache.join("file-index/v1");
        let file = std::fs::read_dir(&directory).unwrap().next().unwrap().unwrap().path();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(base).unwrap();
        MEMORY.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|entry| entry.key != identity);
    }

    #[test]
    fn fresh_index_rewrites_missing_and_expired_disk_entries() {
        if env::var_os("COCKPIT_CACHE_ROOT").is_some() {
            return;
        }
        let base = env::temp_dir().join(format!("cockpit-file-index-refresh-{}", uuid::Uuid::new_v4()));
        let root = base.join("checkout");
        let configured_cache = base.join("cache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&configured_cache).unwrap();
        let files = vec![ContextIndexedFile { path: "a.md".into(), bytes: Some(4) }];
        store(
            &root,
            ContextRootKind::Folder,
            ContextFileIndexSource::Walk,
            false,
            files.clone(),
            &configured_cache,
        );
        let (canonical, dev, ino) = root_identity(&root).unwrap();
        let identity = format!(
            "{}\0{}\0{}\0{}",
            kind_name(ContextRootKind::Folder),
            canonical.to_string_lossy(),
            dev,
            ino
        );
        let cache_file = {
            let (_, _, key) = cache_key(&root, ContextRootKind::Folder, dev, ino).unwrap();
            configured_cache.join("file-index/v1").join(filename(&key))
        };
        std::fs::remove_file(&cache_file).unwrap();
        store(
            &root,
            ContextRootKind::Folder,
            ContextFileIndexSource::Walk,
            false,
            files.clone(),
            &configured_cache,
        );
        assert!(cache_file.is_file(), "a warm memory entry must not hide a missing disk entry");

        let mut persisted: PersistedIndex = serde_json::from_slice(&std::fs::read(&cache_file).unwrap()).unwrap();
        let stale_created_ms = persisted.created_ms.saturating_sub(REWRITE_AFTER.as_millis() + 1_000);
        persisted.created_ms = stale_created_ms;
        std::fs::write(&cache_file, serde_json::to_vec(&persisted).unwrap()).unwrap();
        {
            let mut entries = MEMORY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = entries.iter_mut().find(|entry| entry.key == identity).unwrap();
            entry.created = SystemTime::now() - REWRITE_AFTER - Duration::from_secs(1);
        }
        store(
            &root,
            ContextRootKind::Folder,
            ContextFileIndexSource::Walk,
            false,
            files.clone(),
            &configured_cache,
        );
        let persisted: PersistedIndex = serde_json::from_slice(&std::fs::read(&cache_file).unwrap()).unwrap();
        assert!(persisted.created_ms > stale_created_ms, "an expired persistent entry must be rewritten");

        std::fs::remove_dir_all(&configured_cache).unwrap();
        std::fs::write(&configured_cache, b"cache directory unavailable").unwrap();
        store(
            &root,
            ContextRootKind::Folder,
            ContextFileIndexSource::Walk,
            false,
            files.clone(),
            &configured_cache,
        );
        std::fs::remove_file(&configured_cache).unwrap();
        std::fs::create_dir_all(&configured_cache).unwrap();
        store(
            &root,
            ContextRootKind::Folder,
            ContextFileIndexSource::Walk,
            false,
            files,
            &configured_cache,
        );
        assert!(cache_file.is_file(), "a failed persistence attempt must retry even when memory paths match");
        std::fs::remove_dir_all(base).unwrap();
        MEMORY.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|entry| entry.key != identity);
    }
}

