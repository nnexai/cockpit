use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cockpit_protocol::local_changes::LocalChangeTopic;

use super::{ContextService, AuthorizedRoot, filesystem_identity, open_dir_nofollow_absolute};
use crate::InspectionError;
use crate::local_changes::{LocalChanges, LocalWatchLease, LocalWatchMatch, LocalWatchPlan, LocalWatchRule};

impl ContextService {
    /// Observe only paths already authorized by the Files viewer. The dispatcher
    /// must reauthorize the viewer before delivering each notification.
    pub(crate) async fn watch_files_plan(
        &self, session_id: &str, viewer_id: &str, binding_id: &str,
        root_id: &str, paths: &[String],
    ) -> Result<LocalWatchPlan, InspectionError> {
        let root = self.authorize_files_root(session_id, viewer_id, binding_id, root_id).await?;
        known_files_plan(root, paths)
    }

    /// Register each directory before reading its entries; structural changes
    /// invalidate this epoch and force a fresh discovery.
    pub(crate) async fn watch_index(
        &self, changes: &LocalChanges, session_id: &str, viewer_id: &str,
        binding_id: &str, root_id: &str,
    ) -> Result<LocalWatchLease, InspectionError> {
        let root = self.authorize_files_root(session_id, viewer_id, binding_id, root_id).await?;
        let permit = changes.discovery_permit().await;
        let is_git = super::git_root_matches(&self.configuration, root.canonical_path(), root.root_kind()).await?;
        let before = if is_git { Some(git_inputs(&self.configuration, root.canonical_path()).await?) } else { None };
        let path = root.canonical_path().to_path_buf();
        let kind = root.root_kind();
        let inputs = before.as_ref().map(|snapshot| snapshot.0.clone()).unwrap_or_default();
        let changes = changes.clone();
        let (result, permit) = tokio::task::spawn_blocking(move || {
            (index_watch_inputs(&changes, root, &inputs), permit)
        }).await.map_err(|_| index_unavailable())?;
        let result = result?;
        if super::git_root_matches(&self.configuration, &path, kind).await? != is_git
            || (is_git && Some(git_inputs(&self.configuration, &path).await?) != before) {
            return Err(index_unavailable());
        }
        result.revalidate().map_err(watch_error)?;
        drop(permit);
        Ok(result)
    }
}

pub(crate) fn index_unavailable() -> InspectionError {
    InspectionError::new("local_changes_unavailable", "Automatic index updates are unavailable; refresh to update the index")
}

async fn git_command(configuration: &cockpit_protocol::projects::ProjectConfiguration, root: &Path, args: &[&str]) -> Result<Vec<u8>, InspectionError> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(root).args(args)
        .env("GIT_OPTIONAL_LOCKS", "0").env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0").env("GIT_ASKPASS", "")
        .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE").env_remove("GIT_COMMON_DIR");
    let output = crate::process::run_bounded_command(command, 1024 * 1024, 4096,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64), "context Git watch inputs")
        .await.map_err(|_| index_unavailable())?;
    if !output.status.success() { return Err(index_unavailable()); }
    Ok(output.stdout)
}

/// Resolve dependencies with Git itself, but refuse includes or custom exclude
/// files whose complete authority and lifecycle cannot be established.
async fn git_inputs(configuration: &cockpit_protocol::projects::ProjectConfiguration, root: &Path) -> Result<(Vec<PathBuf>, Vec<u8>), InspectionError> {
    if ["GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG"].iter().any(|name| std::env::var_os(name).is_some()) {
        return Err(index_unavailable());
    }
    let paths = git_command(configuration, root, &["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"]).await?;
    let text = std::str::from_utf8(&paths).map_err(|_| index_unavailable())?;
    let mut lines = text.lines();
    let git_dir = normalized_git_dir(Path::new(lines.next().ok_or_else(index_unavailable)?))?;
    let common = normalized_git_dir(Path::new(lines.next().ok_or_else(index_unavailable)?))?;
    if lines.next().is_some() { return Err(index_unavailable()); }
    let config = git_command(configuration, root, &["config", "--no-includes", "--null", "--show-origin", "--list"]).await?;
    let mut fields = config.split(|byte| *byte == 0).filter(|field| !field.is_empty());
    while let Some(origin) = fields.next() {
        let entry = fields.next().ok_or_else(index_unavailable)?;
        let origin = std::str::from_utf8(origin).map_err(|_| index_unavailable())?;
        let origin = origin.strip_prefix("file:").ok_or_else(index_unavailable)?;
        let origin = Path::new(origin);
        let origin = if origin.is_absolute() { origin.to_path_buf() } else { root.join(origin) };
        let resolved = origin.canonicalize().map_err(|_| index_unavailable())?;
        let allowed = [common.join("config"), git_dir.join("config.worktree")];
        if !allowed.iter().any(|path| path.canonicalize().ok().as_ref() == Some(&resolved)) {
            return Err(index_unavailable());
        }
        let key = entry.split(|byte| *byte == b'\n').next().ok_or_else(index_unavailable)?;
        let key = std::str::from_utf8(key).map_err(|_| index_unavailable())?.to_ascii_lowercase();
        if key.starts_with("include.") || key.starts_with("includeif.") || key == "core.excludesfile" {
            return Err(index_unavailable());
        }
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty())
        .map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(index_unavailable)?;
    if !xdg.is_absolute() { return Err(index_unavailable()); }
    let inputs = vec![root.join(".git"), git_dir.join("index"), common.join("config"),
        git_dir.join("config.worktree"), common.join("info/exclude"), git_dir.join("info/exclude"),
        xdg.join("git/ignore")];
    let mut snapshot = paths;
    snapshot.extend_from_slice(&config);
    Ok((inputs, snapshot))
}

fn normalized_git_dir(path: &Path) -> Result<PathBuf, InspectionError> {
    if !path.is_absolute() { return Err(index_unavailable()); }
    let mut resolved = PathBuf::from("/");
    for component in path.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => {}
            std::path::Component::Normal(name) => {
                resolved.push(name);
                open_dir_nofollow_absolute(&resolved).map_err(|_| index_unavailable())?;
            }
            std::path::Component::ParentDir => {
                if !resolved.pop() { return Err(index_unavailable()); }
            }
            std::path::Component::Prefix(_) => return Err(index_unavailable()),
        }
    }
    Ok(resolved)
}

pub(crate) fn index_watch(changes: &LocalChanges, root: AuthorizedRoot) -> Result<LocalWatchLease, InspectionError> {
    index_watch_inputs(changes, root, &[])
}

fn index_watch_inputs(changes: &LocalChanges, root: AuthorizedRoot, inputs: &[PathBuf]) -> Result<LocalWatchLease, InspectionError> {
    let started = Instant::now();
    let root_path = root.canonical_path().to_path_buf();
    let pinned = Arc::new(Mutex::new(Vec::<(PathBuf, String)>::new()));
    let missing = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let guard_missing = missing.clone();
    let guard_pins = pinned.clone();
    let guard_inputs = inputs.to_vec();
    let check = Arc::new(move || {
        for (path, identity) in guard_pins.lock().map_err(|_| "watch identity unavailable".to_owned())?.iter() {
            let dir = open_dir_nofollow_absolute(path).map_err(|_| "watched directory unavailable".to_owned())?;
            let metadata = dir.dir_metadata().map_err(|_| "watched directory unavailable".to_owned())?;
            if filesystem_identity(&metadata) != *identity {
                return Err("watched directory identity changed".to_owned());
            }
        }
        for path in guard_missing.lock().map_err(|_| "watch identity unavailable".to_owned())?.iter() {
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err("watch directory lifecycle changed".to_owned()),
            }
        }
        for path in &guard_inputs {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() => return Err("Git input became a symbolic link".to_owned()),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Git input unavailable".to_owned()),
            }
        }
        Ok(())
    });
    let plan = LocalWatchPlan::new(Vec::new(), Vec::new(), vec![LocalChangeTopic::Index], check);
    let mut builder = changes.begin(plan).map_err(watch_error)?;
    let mut ancestor = PathBuf::from("/");
    for component in root_path.components() {
        if let std::path::Component::Normal(name) = component {
            let dir = open_dir_nofollow_absolute(&ancestor).map_err(|_| index_unavailable())?;
            let identity = filesystem_identity(&dir.dir_metadata().map_err(|_| index_unavailable())?);
            let child = ancestor.join(name);
            builder.watch_directory(ancestor.clone(), vec![lifecycle(&child)]).map_err(watch_error)?;
            pinned.lock().map_err(|_| index_unavailable())?.push((ancestor.clone(), identity));
            ancestor = child;
        }
    }
    for input in inputs {
        watch_input(&mut builder, input, &pinned, &missing)?;
    }
    let expected = filesystem_identity(&root.dir.dir_metadata().map_err(|_| index_unavailable())?);
    let mut stack = vec![PathBuf::new()];
    let mut scanned = 0usize;
    while let Some(relative) = stack.pop() {
        if started.elapsed() > Duration::from_secs(10) || relative.components().count() >= 64 {
            return Err(index_unavailable());
        }
        let dir = root.resolve_directory(&relative)?;
        let identity = filesystem_identity(&dir.dir_metadata().map_err(|_| index_unavailable())?);
        if relative.as_os_str().is_empty() && identity != expected { return Err(index_unavailable()); }
        let absolute = root_path.join(&relative);
        let mut rules = vec![
            LocalWatchRule { path: absolute.clone(), match_kind: LocalWatchMatch::Children, topics: vec![LocalChangeTopic::Index] },
            LocalWatchRule { path: absolute.clone(), match_kind: LocalWatchMatch::LifecycleDirectoryChildren, topics: Vec::new() },
        ];
        if relative.as_os_str().is_empty() {
            rules.push(LocalWatchRule { path: root_path.join(".git"), match_kind: LocalWatchMatch::InvalidateExact, topics: Vec::new() });
        }
        builder.watch_directory(absolute.clone(), rules).map_err(watch_error)?;
        let opened = open_dir_nofollow_absolute(&absolute).map_err(|_| index_unavailable())?;
        if filesystem_identity(&opened.dir_metadata().map_err(|_| index_unavailable())?) != identity {
            return Err(index_unavailable());
        }
        pinned.lock().map_err(|_| index_unavailable())?.push((absolute, identity));
        for entry in dir.entries().map_err(|_| index_unavailable())? {
            scanned += 1;
            if scanned > 200_000 || started.elapsed() > Duration::from_secs(10) { return Err(index_unavailable()); }
            let entry = entry.map_err(|_| index_unavailable())?;
            let name = entry.file_name();
            let child = relative.join(&name);
            if (inputs.is_empty() && crate::library::folder_io::excluded_source_path(&child))
                || super::reserved_context_path(root.root_kind(), &child).is_some()
                || child.to_str().is_none() || child.as_os_str().as_encoded_bytes().contains(&b'\\') {
                continue;
            }
            let metadata = dir.symlink_metadata(&name).map_err(|_| index_unavailable())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() { stack.push(child); }
        }
    }
    builder.finish().map_err(watch_error)
}

fn watch_error(error: crate::local_changes::LocalWatchError) -> InspectionError {
    InspectionError::new("local_changes_unavailable", error.to_string())
}

fn watch_input(
    builder: &mut crate::local_changes::LocalWatchBuilder,
    input: &Path,
    pinned: &Arc<Mutex<Vec<(PathBuf, String)>>>,
    missing: &Arc<Mutex<Vec<PathBuf>>>,
) -> Result<(), InspectionError> {
    if !input.is_absolute() { return Err(index_unavailable()); }
    let parent = input.parent().ok_or_else(index_unavailable)?;
    let mut current = PathBuf::from("/");
    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            if matches!(component, std::path::Component::RootDir) { continue; }
            return Err(index_unavailable());
        };
        let dir = open_dir_nofollow_absolute(&current).map_err(|_| index_unavailable())?;
        let identity = filesystem_identity(&dir.dir_metadata().map_err(|_| index_unavailable())?);
        let next = current.join(name);
        builder.watch_directory(current.clone(), vec![lifecycle(&next)]).map_err(watch_error)?;
        pinned.lock().map_err(|_| index_unavailable())?.push((current.clone(), identity));
        match open_dir_nofollow_absolute(&next) {
            Ok(_) => current = next,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.lock().map_err(|_| index_unavailable())?.push(next);
                return Ok(());
            }
            Err(_) => return Err(index_unavailable()),
        }
    }
    let dir = open_dir_nofollow_absolute(&current).map_err(|_| index_unavailable())?;
    let identity = filesystem_identity(&dir.dir_metadata().map_err(|_| index_unavailable())?);
    match dir.symlink_metadata(input.file_name().ok_or_else(index_unavailable)?) {
        Ok(metadata) if metadata.file_type().is_symlink() => return Err(index_unavailable()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(index_unavailable()),
    }
    let mut rules = vec![LocalWatchRule { path: input.to_path_buf(), match_kind: LocalWatchMatch::Exact, topics: vec![LocalChangeTopic::Index] }];
    if input.file_name().is_some_and(|name| name == "config" || name == "config.worktree" || name == ".git") {
        rules.push(LocalWatchRule { path: input.to_path_buf(), match_kind: LocalWatchMatch::InvalidateExact, topics: Vec::new() });
    }
    if input.file_name().is_some_and(|name| name == "index") {
        rules.push(LocalWatchRule { path: current.clone(), match_kind: LocalWatchMatch::GitSharedIndexChildren, topics: vec![LocalChangeTopic::Index] });
    }
    builder.watch_directory(current.clone(), rules).map_err(watch_error)?;
    pinned.lock().map_err(|_| index_unavailable())?.push((current, identity));
    Ok(())
}

fn known_files_plan(root: AuthorizedRoot, paths: &[String]) -> Result<LocalWatchPlan, InspectionError> {
    if paths.len() > 128 {
        return Err(InspectionError::new("context_invalid_path", "too many known files"));
    }
    let mut directories = HashSet::new();
    let mut rules = Vec::new();
    let mut identities = Vec::new();
    let mut absent_directories = Vec::new();
    let mut seen = HashSet::new();
    let root_path = root.canonical_path().to_path_buf();

    // Every ancestor is watched for replacement, including the root itself.
    // Opening each component without following links also pins its identity for
    // the registration recheck and for every subsequent event.
    let mut ancestor = PathBuf::from("/");
    pin_directory(&ancestor, &mut directories, &mut identities)?;
    for component in root_path.components() {
        if let std::path::Component::Normal(name) = component {
            ancestor.push(name);
            rules.push(lifecycle(&ancestor));
            pin_directory(&ancestor, &mut directories, &mut identities)?;
        }
    }
    let root_identity = root.dir.dir_metadata().map_err(|error| InspectionError::new("context_stale_root", error.to_string()))?;
    if identities.iter().find(|(path, _)| path == &root_path).map(|(_, id)| id.as_str())
        != Some(filesystem_identity(&root_identity).as_str()) {
        return Err(InspectionError::new("context_stale_root", "context root filesystem identity changed"));
    }
    for path in paths {
        let relative = root.relative_path(path)?;
        if relative.as_os_str().is_empty() || !seen.insert(relative.clone()) {
            return Err(InspectionError::new("context_invalid_path", "known file paths must be nonempty and unique"));
        }
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let mut prefix = PathBuf::new();
        let mut missing = false;
        for component in parent.components() {
            prefix.push(component.as_os_str());
            let absolute = root_path.join(&prefix);
            rules.push(lifecycle(&absolute));
            if missing { continue; }
            match root.resolve_directory(&prefix) {
                Ok(_) => pin_directory(&absolute, &mut directories, &mut identities)?,
                Err(error) if error.code == "context_file_missing" => {
                    absent_directories.push(absolute);
                    missing = true;
                }
                Err(error) => return Err(error),
            }
        }
        if !missing {
            // Check a present leaf without following it; a symlink is not a
            // file capability and must not become one through observation.
            let (dir, leaf) = root.resolve_parent(&relative)?;
            match dir.symlink_metadata(&leaf) {
                Ok(metadata) if metadata.file_type().is_symlink() => return Err(InspectionError::new("context_symlink_refused", "symbolic links are not followed")),
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(InspectionError::new("context_path_unavailable", error.to_string())),
            }
            rules.push(LocalWatchRule { path: root_path.join(&relative), match_kind: LocalWatchMatch::Exact, topics: vec![LocalChangeTopic::Files] });
        }
    }
    let check = Arc::new(move || {
        for (path, identity) in &identities {
            let dir = open_dir_nofollow_absolute(path).map_err(|_| "watched directory is unavailable".to_owned())?;
            let metadata = dir.dir_metadata().map_err(|_| "watched directory is unavailable".to_owned())?;
            if !metadata.is_dir() || filesystem_identity(&metadata) != *identity {
                return Err("watched directory identity changed".to_owned());
            }
        }
        for path in &absent_directories {
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err("watched directory lifecycle changed".to_owned()),
            }
        }
        Ok(())
    });
    check().map_err(|message| InspectionError::new("local_changes_unavailable", message))?;
    Ok(LocalWatchPlan::new(directories.into_iter().collect(), rules, vec![LocalChangeTopic::Files], check))
}

fn lifecycle(path: &Path) -> LocalWatchRule {
    LocalWatchRule { path: path.to_path_buf(), match_kind: LocalWatchMatch::Lifecycle, topics: Vec::new() }
}

fn pin_directory(path: &Path, directories: &mut HashSet<PathBuf>, identities: &mut Vec<(PathBuf, String)>) -> Result<(), InspectionError> {
    if directories.contains(path) { return Ok(()); }
    let dir = open_dir_nofollow_absolute(path).map_err(|error| InspectionError::new("local_changes_unavailable", error.to_string()))?;
    let metadata = dir.dir_metadata().map_err(|error| InspectionError::new("local_changes_unavailable", error.to_string()))?;
    if !metadata.is_dir() { return Err(InspectionError::new("local_changes_unavailable", "watch path is not a directory")); }
    directories.insert(path.to_path_buf());
    identities.push((path.to_path_buf(), filesystem_identity(&metadata)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::fs::Dir;
    use uuid::Uuid;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-context-watch-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn root(&self) -> AuthorizedRoot {
            let dir = Dir::open_ambient_dir(&self.0, cap_std::ambient_authority()).unwrap();
            AuthorizedRoot::library(self.0.clone(), dir, 64).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    fn git(path: &Path, args: &[&str]) {
        let status = std::process::Command::new("git").current_dir(path).args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE").status().unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn git_index_and_ignore_are_watched() {
        let fixture = Fixture::new();
        git(&fixture.0, &["init", "-q"]);
        std::fs::write(fixture.0.join("file"), "content").unwrap();
        let config = cockpit_protocol::projects::ProjectConfiguration::for_tests(&fixture.0);
        let (inputs, _) = git_inputs(&config, &fixture.0).await.unwrap();
        let lease = index_watch_inputs(&LocalChanges::default(), fixture.root(), &inputs).unwrap();
        let mut receiver = lease.receiver();
        let checkpoint = lease.checkpoint();
        git(&fixture.0, &["add", "file"]);
        tokio::time::timeout(Duration::from_secs(3), receiver.changed()).await.unwrap().unwrap();
        assert!(lease.take_since(&checkpoint).topics.contains(&LocalChangeTopic::Index));
        let checkpoint = lease.checkpoint();
        std::fs::write(fixture.0.join(".gitignore"), "*.tmp\n").unwrap();
        tokio::time::timeout(Duration::from_secs(3), receiver.changed()).await.unwrap().unwrap();
        assert!(lease.take_since(&checkpoint).topics.contains(&LocalChangeTopic::Index));
    }

    #[tokio::test]
    async fn git_external_include_is_unavailable() {
        let fixture = Fixture::new();
        git(&fixture.0, &["init", "-q"]);
        git(&fixture.0, &["config", "include.path", "/tmp/external-git-config"]);
        let config = cockpit_protocol::projects::ProjectConfiguration::for_tests(&fixture.0);
        assert!(git_inputs(&config, &fixture.0).await.is_err());
    }

    #[tokio::test]
    async fn linked_worktree_common_config_change_invalidates_epoch() {
        let repository = Fixture::new();
        let linked = Fixture::new();
        git(&repository.0, &["init", "-q"]);
        git(&repository.0, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-qm", "fixture"]);
        git(&repository.0, &["worktree", "add", "-qb", "watch-test", linked.0.to_str().unwrap()]);
        let config = cockpit_protocol::projects::ProjectConfiguration::for_tests(&linked.0);
        let (inputs, _) = git_inputs(&config, &linked.0).await.unwrap();
        let lease = index_watch_inputs(&LocalChanges::default(), linked.root(), &inputs).unwrap();
        let mut receiver = lease.receiver();
        git(&repository.0, &["config", "core.ignorecase", "true"]);
        tokio::time::timeout(Duration::from_secs(3), receiver.changed()).await.unwrap().unwrap();
        assert!(lease.checkpoint().unavailable.is_some());
    }

    #[tokio::test]
    async fn known_file_atomic_replacement_wakes() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("item"), "old").unwrap();
        let plan = known_files_plan(fixture.root(), &["item".into()]).unwrap();
        let lease = LocalChanges::default().subscribe(plan).unwrap();
        let mut rx = lease.receiver();
        let cursor = lease.checkpoint();
        std::fs::write(fixture.0.join("tmp"), "new").unwrap();
        std::fs::rename(fixture.0.join("tmp"), fixture.0.join("item")).unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.changed()).await.unwrap().unwrap();
        assert!(lease.take_since(&cursor).topics.contains(&LocalChangeTopic::Files));
    }

    #[tokio::test]
    async fn new_nested_directory_forces_reconciliation() {
        let fixture = Fixture::new();
        let lease = index_watch(&LocalChanges::default(), fixture.root()).unwrap();
        let mut rx = lease.receiver();
        std::fs::create_dir(fixture.0.join("new")).unwrap();
        std::fs::write(fixture.0.join("new/file"), "new").unwrap();
        tokio::time::timeout(Duration::from_secs(3), rx.changed()).await.unwrap().unwrap();
        assert!(lease.checkpoint().unavailable.is_some());
    }

    #[test]
    fn replaced_root_revalidation_fails() {
        let fixture = Fixture::new();
        let plan = known_files_plan(fixture.root(), &["item".into()]).unwrap();
        let lease = LocalChanges::default().subscribe(plan).unwrap();
        let old = fixture.0.with_extension("old");
        std::fs::rename(&fixture.0, &old).unwrap();
        std::fs::create_dir(&fixture.0).unwrap();
        assert!(lease.revalidate().is_err());
        std::fs::remove_dir_all(old).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn known_file_symlink_escape_is_refused() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        symlink("/tmp", fixture.0.join("escape")).unwrap();
        assert!(known_files_plan(fixture.root(), &["escape/file".into()]).is_err());
    }
}
