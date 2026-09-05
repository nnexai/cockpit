use std::collections::BTreeSet;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::context_assets::{
    ContextSnapshotCopyMode, ContextSnapshotMode, ContextSnapshotResponse,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic, RepositoryCandidate};
use cockpit_protocol::sources::{SourceFreshness, SourceMaterializationStatus};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

use crate::InspectionError;
use crate::process::run_bounded_command;
use crate::project_store::{CompanionManifest, atomic_write_json, read_json_bounded, timestamp};

const MANIFEST_NAME: &str = "context-manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const PENDING_SOURCE_INTENT_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 512;
const MAX_SNAPSHOT_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextManifest {
    schema_version: u32,
    companion_id: String,
    owner_workspace_id: String,
    owner_worktree_path: String,
    primary_repository_identity: String,
    entries: Vec<ContextManifestEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_source_intent: Option<PendingSourceIntent>,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextManifestEntry {
    logical_id: String,
    relative_path: String,
    kind: String,
    source: String,
    generated: bool,
    revision: String,
    content_hash: String,
    bytes: u64,
    copy_mode: String,
    status: String,
    updated_at: String,
    source_repository_id: String,
    source_checkout_path: String,
    source_identity: String,
    source_hash_before: String,
    source_hash_after: String,
}

/// A durable, in-manifest write-ahead record for one generated source
/// replacement. The companion lock allows only one pending source publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSourceIntent {
    schema_version: u32,
    relative_path: String,
    previous_written_hash: Option<String>,
    previous_entry: Option<ContextManifestEntry>,
    new_written_hash: String,
    intended_entry: ContextManifestEntry,
}

/// Materialize one immutable provider payload through the same companion lock,
/// manifest, and no-follow descriptor policy used by repository snapshots.
pub(crate) fn materialize_source_markdown(
    root: &Dir,
    companion_id: &str,
    provider_id: &str,
    provider_instance: &str,
    resource_type: &str,
    canonical_id: &str,
    revision: Option<&str>,
    content_hash: &str,
    markdown: &[u8],
) -> Result<(String, bool), InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let mut manifest = read_manifest(root, companion_id, &association)?;
    recover_pending_source_intent(root, &mut manifest)?;
    let provider = format!(
        "provider-{:x}",
        Sha256::digest(provider_instance.as_bytes())
    );
    let asset = format!("asset-{:x}.md", Sha256::digest(canonical_id.as_bytes()));
    let relative = format!("sources/{provider}/{resource_type}/{asset}");
    let logical_id =
        format!("source:{provider_id}:{provider_instance}:{resource_type}:{canonical_id}");
    // `content_hash` identifies the immutable provider record. The manifest's
    // file hash must instead describe exactly what was written so a later user
    // edit can be distinguished from a legitimate provider refresh.
    let materialized_hash = hash(markdown);
    let previous_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.logical_id == logical_id)
        .cloned();
    let mut replace_owned = false;
    if let Some(entry) = &previous_entry {
        let current = read_stable_source(root, &safe_companion_relative(&entry.relative_path)?)?;
        if current.hash != entry.content_hash {
            return Err(InspectionError::new(
                "source_sync_conflict",
                "a user-modified generated source will not be overwritten",
            ));
        }
        if entry.source_hash_before == content_hash {
            return Ok((entry.relative_path.clone(), false));
        }
        replace_owned = true;
    }
    let path = safe_companion_relative(&relative)?;
    let (parent, leaf) = create_parent(root, &path)?;
    if parent.symlink_metadata(&leaf).is_ok() && !replace_owned {
        return Err(InspectionError::new(
            "source_sync_conflict",
            "an existing companion file is not a matching generated source",
        ));
    }
    let temporary = format!(".source-{}.tmp", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = parent
        .open_with(&temporary, &options)
        .map_err(io_error("source_materialize_failed"))?;
    file.write_all(markdown)
        .map_err(io_error("source_materialize_failed"))?;
    file.sync_all()
        .map_err(io_error("source_materialize_failed"))?;
    let intended_entry = ContextManifestEntry {
        logical_id,
        relative_path: relative.clone(),
        kind: resource_type.to_owned(),
        source: provider_id.to_owned(),
        generated: true,
        revision: revision.unwrap_or("unknown").to_owned(),
        content_hash: materialized_hash.clone(),
        bytes: markdown.len() as u64,
        copy_mode: "write".to_owned(),
        status: "complete".to_owned(),
        updated_at: timestamp(),
        source_repository_id: provider_instance.to_owned(),
        source_checkout_path: String::new(),
        source_identity: canonical_id.to_owned(),
        source_hash_before: content_hash.to_owned(),
        source_hash_after: materialized_hash,
    };
    manifest.pending_source_intent = Some(PendingSourceIntent {
        schema_version: PENDING_SOURCE_INTENT_SCHEMA_VERSION,
        relative_path: relative.clone(),
        previous_written_hash: previous_entry
            .as_ref()
            .map(|entry| entry.content_hash.clone()),
        previous_entry: previous_entry.clone(),
        new_written_hash: intended_entry.content_hash.clone(),
        intended_entry: intended_entry.clone(),
    });
    write_manifest_durable(root, &manifest)?;
    // Recheck after the intent is durable and immediately before replacement.
    // A late user edit abandons this publish; it must never be overwritten.
    if let Err(error) = recheck_source_destination(root, &path, &parent, &leaf, &previous_entry) {
        discard_pending_source_intent(root, &mut manifest, &parent, &temporary)?;
        return Err(error);
    }
    parent
        .rename(&temporary, &parent, &leaf)
        .map_err(io_error("source_materialize_failed"))?;
    sync_directory(&parent).map_err(io_error("source_materialize_failed"))?;
    replace_source_manifest_entry(&mut manifest, intended_entry);
    manifest.pending_source_intent = None;
    manifest.updated_at = timestamp();
    write_manifest_durable(root, &manifest)?;
    Ok((relative, true))
}

/// Resolve a cached source against this exact companion manifest. This does
/// not inspect arbitrary source files: only a manifest-owned generated path is
/// eligible for a materialized result.
pub(crate) fn source_materialization_state(
    root: &Dir,
    companion_id: &str,
    provider_id: &str,
    provider_instance: &str,
    resource_type: &str,
    canonical_id: &str,
    current_hash: &str,
) -> Result<(SourceFreshness, SourceMaterializationStatus, Option<String>), InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let manifest = read_manifest(root, companion_id, &association)?;
    let logical_id =
        format!("source:{provider_id}:{provider_instance}:{resource_type}:{canonical_id}");
    let Some(entry) = manifest
        .entries
        .iter()
        .find(|entry| entry.logical_id == logical_id)
    else {
        return Ok((
            SourceFreshness::Unknown,
            SourceMaterializationStatus::Unchanged,
            None,
        ));
    };
    let path = safe_companion_relative(&entry.relative_path)?;
    let current = match read_stable_source(root, &path) {
        Ok(current) => current,
        Err(_) => {
            return Ok((
                SourceFreshness::Unavailable,
                SourceMaterializationStatus::Failed,
                Some(entry.relative_path.clone()),
            ));
        }
    };
    if current.hash != entry.content_hash {
        return Ok((
            SourceFreshness::Conflict,
            SourceMaterializationStatus::Conflict,
            Some(entry.relative_path.clone()),
        ));
    }
    let freshness = if entry.source_hash_before == current_hash {
        SourceFreshness::Fresh
    } else {
        SourceFreshness::Changed
    };
    Ok((
        freshness,
        SourceMaterializationStatus::Unchanged,
        Some(entry.relative_path.clone()),
    ))
}

#[derive(Debug)]
struct SourceBytes {
    bytes: Vec<u8>,
    hash: String,
    identity: String,
}

#[derive(Debug)]
struct Gitlink {
    path: PathBuf,
    commit: String,
}

struct CompanionLock {
    _file: std::fs::File,
}

/// Snapshot a freshly-resolved catalog repository into an already-authorized
/// companion directory. Both paths are capabilities supplied by ContextService;
/// callers cannot nominate either filesystem root.
pub(crate) async fn snapshot_working_tree(
    configuration: &ProjectConfiguration,
    companion_id: &str,
    companion_root: &Dir,
    companion_path: &Path,
    repository: &RepositoryCandidate,
) -> Result<ContextSnapshotResponse, InspectionError> {
    let source_path = checked_candidate_path(Path::new(&repository.checkout_path))?;
    if paths_overlap(&source_path, companion_path) {
        return Err(InspectionError::new(
            "context_snapshot_nested_destination",
            "the companion destination must not contain, or be contained by, the source checkout",
        ));
    }
    let source_root = open_absolute_dir_nofollow(&source_path).map_err(|error| {
        InspectionError::new(
            "context_snapshot_source_unavailable",
            format!("cannot open source checkout: {error}"),
        )
    })?;
    if !source_root
        .dir_metadata()
        .map_err(|error| {
            InspectionError::new("context_snapshot_source_unavailable", error.to_string())
        })?
        .is_dir()
    {
        return Err(InspectionError::new(
            "context_snapshot_source_unavailable",
            "the resolved repository checkout is not a directory",
        ));
    }

    revalidate_repository(configuration, &source_path, repository, &source_root).await?;

    let mut diagnostics = limits_diagnostics();
    let (paths, gitlinks, excluded, source_head, dirty) =
        git_inventory(configuration, &source_path).await?;
    if paths.len() > MAX_SNAPSHOT_FILES {
        return Err(InspectionError::new(
            "context_snapshot_file_limit",
            "the working tree exceeds Cockpit's snapshot file limit",
        ));
    }

    let _companion_lock = acquire_companion_lock(companion_root)?;
    source_root_revalidate(&source_root, &source_path)?;
    let association = read_companion_association(companion_root)?;
    let mut manifest = read_manifest(companion_root, companion_id, &association)?;
    reject_unmanaged_repositories_root(companion_root, &manifest)?;
    for path in excluded {
        diagnostics.push(diagnostic(
            "context_snapshot_excluded_path",
            "the source path is excluded by snapshot policy",
            Some(&path.to_string_lossy()),
        ));
    }

    let generation = Uuid::new_v4().simple().to_string();
    let staging_name = format!(".context-snapshot-{generation}.tmp");
    companion_root.create_dir(&staging_name).map_err(|error| {
        InspectionError::new(
            "context_snapshot_destination_unavailable",
            format!("cannot create snapshot staging directory: {error}"),
        )
    })?;
    let result = snapshot_into_staging(
        &source_root,
        companion_root,
        &staging_name,
        repository,
        &paths,
        &gitlinks,
        source_head.as_deref(),
        &generation,
        &mut manifest,
        &mut diagnostics,
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = companion_root.remove_dir_all(&staging_name);
            return Err(error);
        }
    };

    let repository_key = repository_key(&repository.repository_id);
    let repositories = ensure_directory(companion_root, "repos")?;
    let repository_dir = ensure_directory(&repositories, &repository_key)?;
    let snapshots = ensure_directory(&repository_dir, "snapshots")?;
    snapshots
        .symlink_metadata(&generation)
        .map(|_| {
            Err(InspectionError::new(
                "context_snapshot_destination_collision",
                "a snapshot generation already exists",
            ))
        })
        .unwrap_or_else(|error| {
            if error.kind() == ErrorKind::NotFound {
                Ok(())
            } else {
                Err(InspectionError::new(
                    "context_snapshot_destination_unavailable",
                    error.to_string(),
                ))
            }
        })?;
    companion_root
        .rename(&staging_name, &snapshots, &generation)
        .map_err(|error| {
            InspectionError::new(
                "context_snapshot_publish_failed",
                format!("cannot publish complete snapshot generation: {error}"),
            )
        })?;

    manifest.entries.extend(result.entries);
    manifest.updated_at = timestamp();
    publish_manifest(companion_root, &snapshots, &generation, &manifest)?;

    let copy_mode = match (result.reflink_files, result.copy_files) {
        (reflinks, 0) if reflinks > 0 => ContextSnapshotCopyMode::Reflink,
        (0, _) => ContextSnapshotCopyMode::Copy,
        _ => ContextSnapshotCopyMode::Mixed,
    };
    diagnostics.push(diagnostic(
        "context_snapshot_copy_mode",
        match copy_mode {
            ContextSnapshotCopyMode::Reflink => "all snapshot files used descriptor-safe reflink copies",
            ContextSnapshotCopyMode::Copy => "reflink was unsupported or cross-device; snapshot files used independent byte copies",
            ContextSnapshotCopyMode::Mixed => "some files used descriptor-safe reflink copies and unsupported files used independent byte copies",
        },
        None,
    ));
    Ok(ContextSnapshotResponse {
        binding_id: String::new(),
        root_id: String::new(),
        repository_id: repository.repository_id.clone(),
        snapshot_path: format!("repos/{repository_key}/snapshots/{generation}"),
        generation,
        mode: ContextSnapshotMode::WorkingTree,
        copy_mode,
        files: result.files,
        bytes: result.bytes,
        source_head,
        dirty,
        diagnostics,
    })
}

fn publish_manifest(
    companion_root: &Dir,
    snapshots: &Dir,
    generation: &str,
    manifest: &ContextManifest,
) -> Result<(), InspectionError> {
    if let Err(error) = atomic_write_json(companion_root, MANIFEST_NAME, manifest) {
        let rollback = snapshots.remove_dir_all(generation).err();
        let detail = rollback.map_or_else(
            || error.to_string(),
            |rollback| format!("{error}; published generation rollback failed: {rollback}"),
        );
        return Err(InspectionError::new(
            "context_snapshot_manifest_failed",
            detail,
        ));
    }
    Ok(())
}

struct StagedSnapshot {
    entries: Vec<ContextManifestEntry>,
    files: u64,
    bytes: u64,
    reflink_files: u64,
    copy_files: u64,
}

#[derive(Debug, Clone, Copy)]
enum FileCopyMode {
    Reflink,
    Copy,
}

#[allow(clippy::too_many_arguments)]
fn snapshot_into_staging(
    source_root: &Dir,
    companion_root: &Dir,
    staging_name: &str,
    repository: &RepositoryCandidate,
    paths: &[PathBuf],
    gitlinks: &[Gitlink],
    source_head: Option<&str>,
    generation: &str,
    _manifest: &mut ContextManifest,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) -> Result<StagedSnapshot, InspectionError> {
    let stage = open_directory(companion_root, Path::new(staging_name))?;
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;
    let mut reflink_files = 0u64;
    let mut copy_files = 0u64;
    for relative in paths {
        let source = match read_stable_source(source_root, relative) {
            Ok(source) => source,
            Err(error) if is_skippable(&error.code) => {
                diagnostics.push(diagnostic(
                    &error.code,
                    &error.message,
                    Some(&relative.to_string_lossy()),
                ));
                continue;
            }
            Err(error) => return Err(error),
        };
        if total_bytes.saturating_add(source.bytes.len() as u64) > MAX_SNAPSHOT_BYTES {
            return Err(InspectionError::new(
                "context_snapshot_byte_limit",
                "the working tree exceeds Cockpit's snapshot byte limit",
            ));
        }
        let copy_mode = write_new_file(&stage, relative, source_root, &source)?;
        match copy_mode {
            FileCopyMode::Reflink => reflink_files += 1,
            FileCopyMode::Copy => copy_files += 1,
        }
        let destination = read_stable_source(&stage, relative)?;
        if destination.hash != source.hash || destination.bytes.len() != source.bytes.len() {
            return Err(InspectionError::new(
                "context_snapshot_hash_mismatch",
                "the published snapshot bytes did not match the source",
            ));
        }
        let final_source = read_stable_source(source_root, relative)?;
        if final_source.hash != source.hash || final_source.identity != source.identity {
            return Err(InspectionError::new(
                "context_snapshot_source_changed",
                "a source file changed during snapshot; retry to capture a complete generation",
            ));
        }
        let relative_path = format!(
            "repos/{}/snapshots/{generation}/{}",
            repository_key(&repository.repository_id),
            relative.to_string_lossy()
        );
        entries.push(ContextManifestEntry {
            logical_id: format!(
                "repository:{}:{}",
                repository.repository_id,
                relative.to_string_lossy()
            ),
            relative_path,
            kind: "repository_snapshot".to_owned(),
            source: "local_repository".to_owned(),
            generated: true,
            revision: source_head.unwrap_or("unborn").to_owned(),
            content_hash: source.hash.clone(),
            bytes: source.bytes.len() as u64,
            copy_mode: match copy_mode {
                FileCopyMode::Reflink => "reflink",
                FileCopyMode::Copy => "copy",
            }
            .to_owned(),
            status: "complete".to_owned(),
            updated_at: timestamp(),
            source_repository_id: repository.repository_id.clone(),
            source_checkout_path: repository.checkout_path.clone(),
            source_identity: source.identity,
            source_hash_before: source.hash.clone(),
            source_hash_after: source.hash,
        });
        total_bytes += entries.last().expect("pushed entry").bytes;
    }
    for gitlink in gitlinks {
        let relative_path = format!(
            "repos/{}/snapshots/{generation}/{}",
            repository_key(&repository.repository_id),
            gitlink.path.to_string_lossy()
        );
        entries.push(ContextManifestEntry {
            logical_id: format!(
                "repository:{}:{}",
                repository.repository_id,
                gitlink.path.to_string_lossy()
            ),
            relative_path,
            kind: "gitlink".to_owned(),
            source: "local_repository".to_owned(),
            generated: true,
            revision: gitlink.commit.clone(),
            content_hash: hash(gitlink.commit.as_bytes()),
            bytes: 0,
            copy_mode: "none".to_owned(),
            status: "skipped_gitlink".to_owned(),
            updated_at: timestamp(),
            source_repository_id: repository.repository_id.clone(),
            source_checkout_path: repository.checkout_path.clone(),
            source_identity: "gitlink".to_owned(),
            source_hash_before: hash(gitlink.commit.as_bytes()),
            source_hash_after: hash(gitlink.commit.as_bytes()),
        });
        diagnostics.push(diagnostic(
            "context_snapshot_gitlink",
            "submodule gitlink was recorded but its working tree was not traversed",
            Some(&gitlink.path.to_string_lossy()),
        ));
    }
    Ok(StagedSnapshot {
        files: reflink_files + copy_files,
        bytes: total_bytes,
        entries,
        reflink_files,
        copy_files,
    })
}

async fn git_inventory(
    configuration: &ProjectConfiguration,
    source: &Path,
) -> Result<
    (
        Vec<PathBuf>,
        Vec<Gitlink>,
        Vec<PathBuf>,
        Option<String>,
        bool,
    ),
    InspectionError,
> {
    let tracked = git_output(
        configuration,
        source,
        &["ls-files", "--stage", "-z", "--cached"],
    )
    .await?;
    let untracked = git_output(
        configuration,
        source,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )
    .await?;
    let status = git_output(
        configuration,
        source,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await?;
    for output in [&tracked, &untracked, &status] {
        if !output.status.success() {
            return Err(InspectionError::new(
                "context_snapshot_git_failed",
                "Git could not inventory the selected repository without mutation",
            ));
        }
    }
    let head = git_output(configuration, source, &["rev-parse", "--verify", "HEAD"]).await?;
    let source_head = if head.status.success() {
        Some(single_line_utf8(
            &head.stdout,
            "context_snapshot_git_output",
        )?)
    } else {
        None
    };
    let mut paths = BTreeSet::new();
    let mut gitlinks = Vec::new();
    let mut excluded = BTreeSet::new();
    for raw in tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let separator = raw.iter().position(|byte| *byte == b'\t').ok_or_else(|| {
            InspectionError::new(
                "context_snapshot_git_output",
                "Git returned an invalid index record",
            )
        })?;
        let (stage, raw_path) = raw.split_at(separator);
        let raw_path = &raw_path[1..];
        let stage = std::str::from_utf8(stage).map_err(|_| {
            InspectionError::new(
                "context_snapshot_git_output",
                "Git returned non-UTF-8 index metadata",
            )
        })?;
        let mut fields = stage.split_whitespace();
        let mode = fields.next().ok_or_else(|| {
            InspectionError::new("context_snapshot_git_output", "Git index mode is missing")
        })?;
        let object = fields.next().ok_or_else(|| {
            InspectionError::new("context_snapshot_git_output", "Git index object is missing")
        })?;
        let text = std::str::from_utf8(raw_path).map_err(|_| {
            InspectionError::new(
                "context_snapshot_non_utf8_path",
                "working-tree snapshot paths must be valid UTF-8",
            )
        })?;
        let path = safe_source_relative(text)?;
        if excluded_source_path(&path) {
            excluded.insert(path);
            continue;
        }
        if mode == "160000" {
            gitlinks.push(Gitlink {
                path,
                commit: object.to_owned(),
            });
        } else {
            paths.insert(path);
        }
    }
    for raw in untracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let text = std::str::from_utf8(raw).map_err(|_| {
            InspectionError::new(
                "context_snapshot_non_utf8_path",
                "working-tree snapshot paths must be valid UTF-8",
            )
        })?;
        let path = safe_source_relative(text)?;
        if excluded_source_path(&path) {
            excluded.insert(path);
            continue;
        }
        paths.insert(path);
    }
    Ok((
        paths.into_iter().collect(),
        gitlinks,
        excluded.into_iter().collect(),
        source_head,
        !status.stdout.is_empty(),
    ))
}

async fn git_output(
    configuration: &ProjectConfiguration,
    source: &Path,
    args: &[&str],
) -> Result<std::process::Output, InspectionError> {
    let mut command = Command::new("git");
    command
        .current_dir(source)
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY");
    run_bounded_command(
        command,
        configuration.limits.git_output_bytes as usize,
        configuration.limits.git_output_bytes as usize,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "context_snapshot_git",
    )
    .await
}

fn read_companion_association(root: &Dir) -> Result<CompanionManifest, InspectionError> {
    let association: CompanionManifest =
        read_json_bounded(root, "manifest.json", MAX_MANIFEST_BYTES).map_err(|_| {
            InspectionError::new(
                "context_snapshot_companion_unavailable",
                "the authorized companion has no readable durable association",
            )
        })?;
    if association.ownership != "cockpit" {
        return Err(InspectionError::new(
            "context_snapshot_companion_unavailable",
            "the companion association is not Cockpit-owned",
        ));
    }
    Ok(association)
}

fn read_manifest(
    root: &Dir,
    companion_id: &str,
    association: &CompanionManifest,
) -> Result<ContextManifest, InspectionError> {
    match root.symlink_metadata(MANIFEST_NAME) {
        Ok(_) => {
            let manifest: ContextManifest =
                read_json_bounded(root, MANIFEST_NAME, MAX_MANIFEST_BYTES)?;
            if manifest.schema_version != MANIFEST_SCHEMA_VERSION
                || manifest.companion_id != companion_id
                || manifest.owner_workspace_id != association.herdr_workspace_id
                || manifest.owner_worktree_path != association.checkout_path
                || manifest.primary_repository_identity != association.repository_key
            {
                return Err(InspectionError::new(
                    "context_manifest_owner_mismatch",
                    "the context manifest is not owned by this authorized companion",
                ));
            }
            Ok(manifest)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(ContextManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            companion_id: companion_id.to_owned(),
            owner_workspace_id: association.herdr_workspace_id.clone(),
            owner_worktree_path: association.checkout_path.clone(),
            primary_repository_identity: association.repository_key.clone(),
            entries: Vec::new(),
            pending_source_intent: None,
            updated_at: timestamp(),
        }),
        Err(error) => Err(InspectionError::new(
            "context_manifest_unavailable",
            error.to_string(),
        )),
    }
}

fn recover_pending_source_intent(
    root: &Dir,
    manifest: &mut ContextManifest,
) -> Result<(), InspectionError> {
    let Some(intent) = manifest.pending_source_intent.clone() else {
        return Ok(());
    };
    validate_pending_source_intent(&intent)?;
    let path = safe_companion_relative(&intent.relative_path)?;
    let current = match read_stable_source(root, &path) {
        Ok(current) => current,
        Err(error)
            if error.code == "context_snapshot_file_missing" && intent.previous_entry.is_none() =>
        {
            manifest.pending_source_intent = None;
            return write_manifest_durable(root, manifest);
        }
        Err(_) => return clear_pending_source_conflict(root, manifest),
    };
    let current_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.logical_id == intent.intended_entry.logical_id);
    if current_entry == Some(&intent.intended_entry) {
        if current.hash != intent.new_written_hash {
            return clear_pending_source_conflict(root, manifest);
        }
        manifest.pending_source_intent = None;
        return write_manifest_durable(root, manifest);
    }
    let previous_matches = match (&intent.previous_entry, current_entry) {
        (None, None) => true,
        (Some(previous), Some(current_entry)) => previous == current_entry,
        _ => false,
    };
    if !previous_matches {
        return clear_pending_source_conflict(root, manifest);
    }
    if current.hash == intent.new_written_hash {
        replace_source_manifest_entry(manifest, intent.intended_entry);
        manifest.pending_source_intent = None;
        manifest.updated_at = timestamp();
        return write_manifest_durable(root, manifest);
    }
    if intent
        .previous_written_hash
        .as_deref()
        .is_some_and(|expected| current.hash == expected)
    {
        // The intent persisted but the rename did not. It is safe to discard
        // it because the prior manifest and bytes still agree exactly.
        manifest.pending_source_intent = None;
        return write_manifest_durable(root, manifest);
    }
    clear_pending_source_conflict(root, manifest)
}

fn recheck_source_destination(
    root: &Dir,
    path: &Path,
    parent: &Dir,
    leaf: &Path,
    previous_entry: &Option<ContextManifestEntry>,
) -> Result<(), InspectionError> {
    let Some(previous) = previous_entry else {
        return match parent.symlink_metadata(leaf) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(InspectionError::new(
                "source_sync_conflict",
                "an existing companion file is not a matching generated source",
            )),
            Err(error) => Err(InspectionError::new(
                "source_materialize_failed",
                error.to_string(),
            )),
        };
    };
    let current = read_stable_source(root, path).map_err(|_| {
        InspectionError::new(
            "source_sync_conflict",
            "a generated source changed while Cockpit was refreshing it",
        )
    })?;
    if current.hash != previous.content_hash {
        return Err(InspectionError::new(
            "source_sync_conflict",
            "a user-modified generated source will not be overwritten",
        ));
    }
    Ok(())
}

fn discard_pending_source_intent(
    root: &Dir,
    manifest: &mut ContextManifest,
    parent: &Dir,
    temporary: &str,
) -> Result<(), InspectionError> {
    manifest.pending_source_intent = None;
    write_manifest_durable(root, manifest)?;
    match parent.remove_file(temporary) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(InspectionError::new(
            "source_materialize_failed",
            error.to_string(),
        )),
    }
}

fn clear_pending_source_conflict(
    root: &Dir,
    manifest: &mut ContextManifest,
) -> Result<(), InspectionError> {
    manifest.pending_source_intent = None;
    write_manifest_durable(root, manifest)?;
    Err(source_publish_conflict())
}

fn validate_pending_source_intent(intent: &PendingSourceIntent) -> Result<(), InspectionError> {
    if intent.schema_version != PENDING_SOURCE_INTENT_SCHEMA_VERSION
        || intent.relative_path != intent.intended_entry.relative_path
        || intent.new_written_hash != intent.intended_entry.content_hash
        || intent
            .previous_entry
            .as_ref()
            .map(|entry| entry.content_hash.as_str())
            != intent.previous_written_hash.as_deref()
    {
        return Err(InspectionError::new(
            "source_publish_intent_invalid",
            "the pending source publish intent is not internally consistent",
        ));
    }
    if let Some(previous) = &intent.previous_entry {
        if previous.logical_id != intent.intended_entry.logical_id
            || previous.relative_path != intent.relative_path
        {
            return Err(InspectionError::new(
                "source_publish_intent_invalid",
                "the pending source publish intent does not describe one owned path",
            ));
        }
    }
    safe_companion_relative(&intent.relative_path)?;
    Ok(())
}

fn replace_source_manifest_entry(manifest: &mut ContextManifest, intended: ContextManifestEntry) {
    manifest
        .entries
        .retain(|entry| entry.logical_id != intended.logical_id);
    manifest.entries.push(intended);
}

fn source_publish_conflict() -> InspectionError {
    InspectionError::new(
        "source_sync_conflict",
        "a pending generated source publish does not match its recorded bytes",
    )
}

fn write_manifest_durable(root: &Dir, manifest: &ContextManifest) -> Result<(), InspectionError> {
    let temporary = format!(".context-manifest-{}.tmp", Uuid::new_v4());
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| InspectionError::new("source_manifest_failed", error.to_string()))?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = root
        .open_with(&temporary, &options)
        .map_err(io_error("source_manifest_failed"))?;
    file.write_all(&bytes)
        .map_err(io_error("source_manifest_failed"))?;
    file.sync_all()
        .map_err(io_error("source_manifest_failed"))?;
    match root.symlink_metadata(MANIFEST_NAME) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(InspectionError::new(
                "source_manifest_failed",
                "the context manifest destination is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(InspectionError::new(
                "source_manifest_failed",
                error.to_string(),
            ));
        }
    }
    root.rename(&temporary, root, MANIFEST_NAME)
        .map_err(io_error("source_manifest_failed"))?;
    sync_directory(root).map_err(io_error("source_manifest_failed"))?;
    Ok(())
}

fn sync_directory(dir: &Dir) -> std::io::Result<()> {
    dir.open(Path::new("."))?.sync_all()
}

fn reject_unmanaged_repositories_root(
    root: &Dir,
    manifest: &ContextManifest,
) -> Result<(), InspectionError> {
    match root.symlink_metadata("repos") {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(InspectionError::new(
                "context_snapshot_destination_unavailable",
                "the companion repositories path is not a real directory",
            ))
        }
        Ok(_) if manifest.entries.is_empty() => Err(InspectionError::new(
            "context_snapshot_destination_conflict",
            "the companion repositories path exists without Cockpit manifest ownership",
        )),
        Ok(_) | Err(_) => Ok(()),
    }
}

fn read_stable_source(root: &Dir, relative: &Path) -> Result<SourceBytes, InspectionError> {
    let (parent, leaf) = resolve_parent(root, relative)?;
    let before = read_regular(&parent, &leaf)?;
    let after = read_regular(&parent, &leaf)?;
    if before.identity != after.identity || before.hash != after.hash {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed during snapshot; retry to capture a complete generation",
        ));
    }
    Ok(before)
}

fn read_regular(parent: &Dir, leaf: &Path) -> Result<SourceBytes, InspectionError> {
    let metadata = parent.symlink_metadata(leaf).map_err(|error| {
        InspectionError::new(
            if error.kind() == ErrorKind::NotFound {
                "context_snapshot_file_missing"
            } else {
                "context_snapshot_file_unavailable"
            },
            error.to_string(),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "context_snapshot_symlink",
            "symbolic links are recorded as skipped",
        ));
    }
    if !metadata.is_file() {
        return Err(InspectionError::new(
            "context_snapshot_special_file",
            "only regular files are eligible for snapshots",
        ));
    }
    if metadata.len() > MAX_SNAPSHOT_FILE_BYTES as u64 {
        return Err(InspectionError::new(
            "context_snapshot_file_bytes",
            "a source file exceeds Cockpit's snapshot file limit",
        ));
    }
    if hardlinked(&metadata) {
        return Err(InspectionError::new(
            "context_snapshot_hardlink",
            "hardlinked source files are not copied into snapshots",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = parent.open_with(leaf, &options).map_err(|error| {
        InspectionError::new("context_snapshot_file_unavailable", error.to_string())
    })?;
    let opened = file.metadata().map_err(|error| {
        InspectionError::new("context_snapshot_file_unavailable", error.to_string())
    })?;
    if opened.file_type().is_symlink()
        || !opened.is_file()
        || identity(&opened) != identity(&metadata)
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed while opening",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_SNAPSHOT_FILE_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            InspectionError::new("context_snapshot_file_unavailable", error.to_string())
        })?;
    if bytes.len() > MAX_SNAPSHOT_FILE_BYTES {
        return Err(InspectionError::new(
            "context_snapshot_file_bytes",
            "a source file grew beyond Cockpit's snapshot file limit",
        ));
    }
    if native_executable(&opened, &bytes) {
        return Err(InspectionError::new(
            "context_snapshot_native_binary",
            "native executable binaries are not copied into snapshots",
        ));
    }
    Ok(SourceBytes {
        hash: hash(&bytes),
        bytes,
        identity: identity(&opened),
    })
}

fn write_new_file(
    root: &Dir,
    relative: &Path,
    source_root: &Dir,
    source: &SourceBytes,
) -> Result<FileCopyMode, InspectionError> {
    let (parent, leaf) = create_parent(root, relative)?;
    if parent.symlink_metadata(&leaf).is_ok() {
        return Err(InspectionError::new(
            "context_snapshot_destination_collision",
            "a snapshot destination path already exists",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = parent.open_with(&leaf, &options).map_err(|error| {
        InspectionError::new(
            "context_snapshot_destination_unavailable",
            error.to_string(),
        )
    })?;
    let source_file = open_source_for_clone(source_root, relative, &source.identity)?;
    let mode = match reflink(&file, &source_file) {
        Ok(()) => FileCopyMode::Reflink,
        Err(error) if reflink_fallback(&error) => {
            file.write_all(&source.bytes).map_err(|error| {
                InspectionError::new(
                    "context_snapshot_destination_unavailable",
                    error.to_string(),
                )
            })?;
            FileCopyMode::Copy
        }
        Err(error) => {
            return Err(InspectionError::new(
                "context_snapshot_reflink_failed",
                format!("reflink could not create an independent snapshot file: {error}"),
            ));
        }
    };
    file.sync_all().map_err(|error| {
        InspectionError::new(
            "context_snapshot_destination_unavailable",
            error.to_string(),
        )
    })?;
    Ok(mode)
}

fn open_source_for_clone(
    root: &Dir,
    relative: &Path,
    expected_identity: &str,
) -> Result<cap_std::fs::File, InspectionError> {
    let (parent, leaf) = resolve_parent(root, relative)?;
    let metadata = parent
        .symlink_metadata(&leaf)
        .map_err(io_error("context_snapshot_source_changed"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || identity(&metadata) != expected_identity
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed before reflink",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = parent
        .open_with(&leaf, &options)
        .map_err(io_error("context_snapshot_source_changed"))?;
    let opened = file
        .metadata()
        .map_err(io_error("context_snapshot_source_changed"))?;
    if opened.file_type().is_symlink()
        || !opened.is_file()
        || identity(&opened) != expected_identity
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed while opening for reflink",
        ));
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn reflink(destination: &cap_std::fs::File, source: &cap_std::fs::File) -> std::io::Result<()> {
    rustix::fs::ioctl_ficlone(destination, source).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "linux"))]
fn reflink(_destination: &cap_std::fs::File, _source: &cap_std::fs::File) -> std::io::Result<()> {
    Err(std::io::Error::from(ErrorKind::Unsupported))
}

fn reflink_fallback(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Unsupported | ErrorKind::InvalidInput
    ) || error.raw_os_error() == Some(nix::libc::EXDEV)
        || error.raw_os_error() == Some(nix::libc::ENOTTY)
        || error.raw_os_error() == Some(nix::libc::EOPNOTSUPP)
}

fn create_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_snapshot_path", "snapshot path has no file name")
    })?;
    let mut current = root
        .try_clone()
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_snapshot_path",
                "snapshot path escapes its root",
            ));
        };
        current = ensure_directory(
            &current,
            name.to_str().ok_or_else(|| {
                InspectionError::new("context_snapshot_path", "snapshot paths must be UTF-8")
            })?,
        )?;
    }
    Ok((current, leaf.into()))
}

fn ensure_directory(root: &Dir, name: &str) -> Result<Dir, InspectionError> {
    match root.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(InspectionError::new(
                "context_snapshot_destination_unavailable",
                error.to_string(),
            ));
        }
    }
    let metadata = root
        .symlink_metadata(name)
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(InspectionError::new(
            "context_snapshot_destination_unavailable",
            "snapshot directory is not a real directory",
        ));
    }
    root.open_dir_nofollow(Path::new(name))
        .map_err(io_error("context_snapshot_destination_unavailable"))
}

fn open_directory(root: &Dir, path: &Path) -> Result<Dir, InspectionError> {
    let mut current = root
        .try_clone()
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_snapshot_path",
                "snapshot path escapes its root",
            ));
        };
        current = current
            .open_dir_nofollow(Path::new(name))
            .map_err(io_error("context_snapshot_destination_unavailable"))?;
    }
    Ok(current)
}

fn resolve_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_snapshot_path", "snapshot path has no file name")
    })?;
    Ok((
        open_directory(root, relative.parent().unwrap_or_else(|| Path::new("")))?,
        leaf.into(),
    ))
}

fn safe_source_relative(value: &str) -> Result<PathBuf, InspectionError> {
    safe_relative(value)
}

fn safe_companion_relative(value: &str) -> Result<PathBuf, InspectionError> {
    safe_relative(value)
}

fn safe_relative(value: &str) -> Result<PathBuf, InspectionError> {
    let candidate = Path::new(value);
    if value.is_empty() || candidate.is_absolute() || value.contains('\0') {
        return Err(InspectionError::new(
            "context_snapshot_path",
            "snapshot paths must be bounded relative paths",
        ));
    }
    let mut result = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => result.push(part),
            _ => {
                return Err(InspectionError::new(
                    "context_snapshot_path",
                    "snapshot path escapes its root",
                ));
            }
        }
    }
    Ok(result)
}

fn excluded_source_path(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return true;
        };
        matches!(
            name.to_str(),
            Some(".git" | "node_modules" | "target" | "build" | "dist" | ".next")
        )
    })
}

fn is_skippable(code: &str) -> bool {
    matches!(
        code,
        "context_snapshot_symlink"
            | "context_snapshot_special_file"
            | "context_snapshot_hardlink"
            | "context_snapshot_native_binary"
            | "context_snapshot_file_missing"
    )
}

fn checked_candidate_path(path: &Path) -> Result<PathBuf, InspectionError> {
    if !path.is_absolute() {
        return Err(InspectionError::new(
            "context_snapshot_source_unavailable",
            "repository checkout path is not absolute",
        ));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::Prefix(_)) {
            return Err(InspectionError::new(
                "context_snapshot_source_unavailable",
                "repository checkout path is not a canonical catalog path",
            ));
        }
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        InspectionError::new("context_snapshot_source_unavailable", error.to_string())
    })?;
    if metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "context_snapshot_source_unavailable",
            "repository checkout symlinks are not accepted as snapshot roots",
        ));
    }
    if !metadata.is_dir() {
        return Err(InspectionError::new(
            "context_snapshot_source_unavailable",
            "repository checkout is not a directory",
        ));
    }
    Ok(path.to_path_buf())
}

async fn revalidate_repository(
    configuration: &ProjectConfiguration,
    source: &Path,
    repository: &RepositoryCandidate,
    source_root: &Dir,
) -> Result<(), InspectionError> {
    let top = git_output(configuration, source, &["rev-parse", "--show-toplevel"]).await?;
    if !top.status.success() {
        return Err(InspectionError::new(
            "context_snapshot_repository_changed",
            "the selected catalog repository is no longer a usable Git checkout",
        ));
    }
    let reported = single_line_utf8(&top.stdout, "context_snapshot_git_output")?;
    if Path::new(&reported) != Path::new(&repository.checkout_path) {
        return Err(InspectionError::new(
            "context_snapshot_repository_changed",
            "Git checkout identity no longer matches the fresh catalog candidate",
        ));
    }
    source_root_revalidate(source_root, source)
}

fn source_root_revalidate(source_root: &Dir, source: &Path) -> Result<(), InspectionError> {
    let opened = source_root.dir_metadata().map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    let reopened = open_absolute_dir_nofollow(source).map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    let current = reopened.dir_metadata().map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    if !opened.is_dir()
        || !current.is_dir()
        || object_identity(&opened) != object_identity(&current)
    {
        return Err(InspectionError::new(
            "context_snapshot_repository_changed",
            "the selected checkout changed after catalog resolution",
        ));
    }
    Ok(())
}

fn acquire_companion_lock(root: &Dir) -> Result<CompanionLock, InspectionError> {
    const LOCK_NAME: &str = ".context-assets.lock";
    match root.symlink_metadata(LOCK_NAME) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(InspectionError::new(
                "context_snapshot_lock_unavailable",
                "the companion snapshot lock path is not a regular file",
            ));
        }
        Ok(_) | Err(_) => {}
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let file = root
        .open_with(LOCK_NAME, &options)
        .map_err(io_error("context_snapshot_lock_unavailable"))?
        .into_std();
    file.lock_exclusive()
        .map_err(io_error("context_snapshot_lock_unavailable"))?;
    Ok(CompanionLock { _file: file })
}

fn open_absolute_dir_nofollow(path: &Path) -> std::io::Result<Dir> {
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => dir = dir.open_dir_nofollow(Path::new(name))?,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe absolute path",
                ));
            }
        }
    }
    Ok(dir)
}

fn paths_overlap(first: &Path, second: &Path) -> bool {
    first == second || first.strip_prefix(second).is_ok() || second.strip_prefix(first).is_ok()
}

fn repository_key(repository_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repository_id.as_bytes());
    format!("repo-{:x}", hasher.finalize())
}

fn hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime_nsec()
        );
    }
    #[cfg(not(unix))]
    {
        format!("{}:{:?}", metadata.len(), metadata.modified().ok())
    }
}

fn object_identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return format!("{}:{}", metadata.dev(), metadata.ino());
    }
    #[cfg(not(unix))]
    {
        format!("{}", metadata.len())
    }
}

fn hardlinked(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return metadata.nlink() > 1;
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn native_executable(metadata: &Metadata, bytes: &[u8]) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.mode() & 0o111 == 0 {
            return false;
        }
        return bytes.starts_with(b"\x7fELF")
            || matches!(
                bytes.get(..4),
                Some(
                    [0xfe, 0xed, 0xfa, 0xce]
                        | [0xfe, 0xed, 0xfa, 0xcf]
                        | [0xcf, 0xfa, 0xed, 0xfe]
                        | [0xce, 0xfa, 0xed, 0xfe]
                )
            );
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, bytes);
        false
    }
}

fn single_line_utf8(bytes: &[u8], code: &str) -> Result<String, InspectionError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| InspectionError::new(code, "Git returned non-UTF-8 output"))?
        .trim();
    if text.is_empty() || text.lines().count() != 1 || text.chars().any(char::is_control) {
        return Err(InspectionError::new(
            code,
            "Git returned an invalid revision",
        ));
    }
    Ok(text.to_owned())
}

fn limits_diagnostics() -> Vec<ProjectDiagnostic> {
    vec![diagnostic(
        "context_snapshot_limits",
        "snapshot limits: 512 files, 4 MiB per file, 32 MiB total; Git output and timeout use configured project limits",
        None,
    )]
}

fn diagnostic(code: &str, message: &str, path: Option<&str>) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        path: path.map(str::to_owned),
    }
}

fn io_error(code: &'static str) -> impl FnOnce(std::io::Error) -> InspectionError {
    move |error| InspectionError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::fs::Dir;
    use cockpit_protocol::projects::{ProjectLimits, ProjectProvider};
    use std::collections::BTreeMap;
    use std::fs;
    use std::process::Command as ProcessCommand;

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("cockpit-context-assets-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temporary directory");
        path
    }

    fn configuration() -> ProjectConfiguration {
        ProjectConfiguration {
            version: 1,
            repository_roots: Vec::new(),
            worktree_root: "worktrees".to_owned(),
            companion_root: "companions".to_owned(),
            state_root: "state".to_owned(),
            branch_template: "{repo}/{task_id}".to_owned(),
            checkout_template: "{repo}-{task_id}".to_owned(),
            providers: vec![ProjectProvider {
                id: "test".to_owned(),
                base_url: "https://example.test".to_owned(),
                executable: "tea".to_owned(),
                login: None,
            }],
            limits: ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 16,
                git_timeout_ms: 2_000,
                git_output_bytes: 64 * 1024,
                operation_timeout_ms: 2_000,
                context_preview_bytes: 1024,
                context_preview_lines: 100,
                context_directory_entries: 100,
                context_tree_depth: 16,
            },
            origins: BTreeMap::new(),
        }
    }

    fn association(workspace: &str, checkout: &Path) -> CompanionManifest {
        CompanionManifest {
            schema_version: 1,
            cockpit_operation_id: "op-test".to_owned(),
            herdr_session_identity: "session-test".to_owned(),
            herdr_workspace_id: workspace.to_owned(),
            repository_key: "primary-repository".to_owned(),
            repository_root: checkout.to_string_lossy().into_owned(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            artifact: None,
            created_at: timestamp(),
            updated_at: timestamp(),
            ownership: "cockpit".to_owned(),
        }
    }

    fn stage_interrupted_source_refresh(
        dir: &Dir,
        manifest: &mut ContextManifest,
        previous: ContextManifestEntry,
        markdown: &[u8],
    ) -> ContextManifestEntry {
        let mut intended = previous.clone();
        intended.revision = "2".to_owned();
        intended.content_hash = hash(markdown);
        intended.bytes = markdown.len() as u64;
        intended.updated_at = timestamp();
        intended.source_hash_before = "two".to_owned();
        intended.source_hash_after = intended.content_hash.clone();
        manifest.pending_source_intent = Some(PendingSourceIntent {
            schema_version: PENDING_SOURCE_INTENT_SCHEMA_VERSION,
            relative_path: intended.relative_path.clone(),
            previous_written_hash: Some(previous.content_hash.clone()),
            previous_entry: Some(previous),
            new_written_hash: intended.content_hash.clone(),
            intended_entry: intended.clone(),
        });
        write_manifest_durable(dir, manifest).expect("persist source publish intent");
        intended
    }

    fn rename_source_for_interrupted_publish(dir: &Dir, relative: &str, markdown: &[u8]) {
        let path = safe_companion_relative(relative).expect("safe source path");
        let (parent, leaf) = create_parent(dir, &path).expect("source parent");
        let temporary = format!(".interrupted-source-{}.tmp", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(cap_fs_ext::FollowSymlinks::No);
        let mut file = parent
            .open_with(&temporary, &options)
            .expect("temporary source");
        file.write_all(markdown).expect("write temporary source");
        file.sync_all().expect("sync temporary source");
        parent
            .rename(&temporary, &parent, &leaf)
            .expect("replace source atomically");
    }

    fn candidate(checkout: &Path) -> RepositoryCandidate {
        RepositoryCandidate {
            repository_id: "repository-test".to_owned(),
            name: "repository-test".to_owned(),
            root: checkout.to_string_lossy().into_owned(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            common_dir: checkout.join(".git").to_string_lossy().into_owned(),
            branch: Some("main".to_owned()),
            is_linked_worktree: false,
            is_detached: false,
            provenance: "test".to_owned(),
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let status = ProcessCommand::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn source_policy_rejects_escape_and_excluded_paths() {
        assert!(safe_source_relative("../escape").is_err());
        assert!(excluded_source_path(
            &safe_source_relative(".git/config").expect("relative")
        ));
        assert!(excluded_source_path(
            &safe_source_relative("node_modules/pkg/index.js").expect("relative")
        ));
        assert_eq!(
            safe_source_relative("src/lib.rs")
                .expect("safe")
                .to_string_lossy(),
            "src/lib.rs"
        );
    }

    #[test]
    fn copy_creates_independent_regular_file() {
        let root = temp_dir("copy");
        fs::create_dir(root.join("src")).expect("source directory");
        fs::write(root.join("src/main.rs"), b"fn main() {}\n").expect("source");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        root_dir.create_dir("stage").expect("stage");
        let stage = open_directory(&root_dir, Path::new("stage")).expect("open stage");
        let source = read_stable_source(&root_dir, Path::new("src/main.rs")).expect("read source");
        write_new_file(&stage, Path::new("src/main.rs"), &root_dir, &source).expect("copy");
        let copied = read_stable_source(&stage, Path::new("src/main.rs")).expect("read copy");
        assert_eq!(copied.bytes, b"fn main() {}\n");
        assert!(!hardlinked(
            &stage.symlink_metadata("src/main.rs").expect("metadata")
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn changed_source_rejects_the_pending_copy() {
        let root = temp_dir("changed");
        fs::write(root.join("source.rs"), b"before").expect("source");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        root_dir.create_dir("stage").expect("stage");
        let stage = open_directory(&root_dir, Path::new("stage")).expect("open stage");
        let source = read_stable_source(&root_dir, Path::new("source.rs")).expect("read source");
        fs::write(root.join("source.rs"), b"after").expect("mutate source");
        assert_eq!(
            write_new_file(&stage, Path::new("source.rs"), &root_dir, &source)
                .expect_err("changed source must fail")
                .code,
            "context_snapshot_source_changed"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn manifest_owner_mismatch_is_refused_and_publish_failure_rolls_back() {
        let root = temp_dir("manifest");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let first = association("workspace-a", Path::new("/worktree-a"));
        atomic_write_json(&root_dir, "manifest.json", &first).expect("association");
        let manifest = ContextManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            companion_id: "companion-a".to_owned(),
            owner_workspace_id: "workspace-b".to_owned(),
            owner_worktree_path: "/worktree-a".to_owned(),
            primary_repository_identity: "primary-repository".to_owned(),
            entries: Vec::new(),
            pending_source_intent: None,
            updated_at: timestamp(),
        };
        atomic_write_json(&root_dir, MANIFEST_NAME, &manifest).expect("content manifest");
        assert_eq!(
            read_manifest(&root_dir, "companion-a", &first)
                .expect_err("wrong owner")
                .code,
            "context_manifest_owner_mismatch"
        );

        root_dir
            .remove_file(MANIFEST_NAME)
            .expect("remove manifest");
        root_dir
            .create_dir(MANIFEST_NAME)
            .expect("make manifest path fail");
        let snapshots = ensure_directory(
            &ensure_directory(
                &ensure_directory(&root_dir, "repos").expect("repos"),
                "repo",
            )
            .expect("repo"),
            "snapshots",
        )
        .expect("snapshots");
        snapshots.create_dir("generation").expect("generation");
        let error = publish_manifest(&root_dir, &snapshots, "generation", &manifest)
            .expect_err("manifest publish fails");
        assert_eq!(error.code, "context_snapshot_manifest_failed");
        assert!(
            matches!(snapshots.symlink_metadata("generation"), Err(error) if error.kind() == ErrorKind::NotFound)
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn replaced_catalog_checkout_symlink_is_refused_before_opening() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("candidate-symlink");
        let replacement = temp_dir("candidate-replacement");
        let link = root.join("checkout");
        symlink(&replacement, &link).expect("symlink");
        assert_eq!(
            checked_candidate_path(&link)
                .expect_err("catalog checkout replacement")
                .code,
            "context_snapshot_source_unavailable"
        );
        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_dir_all(replacement).expect("cleanup replacement");
    }

    #[tokio::test]
    async fn working_tree_snapshot_keeps_dirty_and_untracked_bytes_but_omits_ignored() {
        let source = temp_dir("repository");
        let companion = temp_dir("companion");
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        git(&source, &["config", "user.name", "Cockpit test"]);
        fs::write(source.join("tracked.txt"), b"clean\n").expect("tracked");
        fs::write(source.join(".gitignore"), b"ignored.txt\n").expect("ignore");
        fs::create_dir(source.join("target")).expect("excluded directory");
        fs::write(source.join("target/tracked.cache"), b"cached\n").expect("excluded tracked");
        git(
            &source,
            &["add", "tracked.txt", ".gitignore", "target/tracked.cache"],
        );
        git(&source, &["commit", "--quiet", "-m", "initial"]);
        fs::write(source.join("tracked.txt"), b"dirty\n").expect("dirty");
        fs::write(source.join("untracked.txt"), b"untracked\n").expect("untracked");
        fs::write(source.join("ignored.txt"), b"ignored\n").expect("ignored");

        let companion_dir = Dir::open_ambient_dir(&companion, cap_std::ambient_authority())
            .expect("open companion");
        atomic_write_json(
            &companion_dir,
            "manifest.json",
            &association("workspace-a", &source),
        )
        .expect("association");
        let response = snapshot_working_tree(
            &configuration(),
            "companion-a",
            &companion_dir,
            &companion,
            &candidate(&source),
        )
        .await
        .expect("snapshot");
        assert!(response.dirty);
        assert!(matches!(
            response.copy_mode,
            ContextSnapshotCopyMode::Reflink | ContextSnapshotCopyMode::Copy
        ));
        let snapshot = companion.join(&response.snapshot_path);
        assert_eq!(
            fs::read(snapshot.join("tracked.txt")).expect("dirty copy"),
            b"dirty\n"
        );
        assert_eq!(
            fs::read(snapshot.join("untracked.txt")).expect("untracked copy"),
            b"untracked\n"
        );
        assert!(!snapshot.join("ignored.txt").exists());
        assert!(!snapshot.join("target/tracked.cache").exists());
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "context_snapshot_excluded_path")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(
                fs::metadata(source.join("tracked.txt"))
                    .expect("source metadata")
                    .ino(),
                fs::metadata(snapshot.join("tracked.txt"))
                    .expect("snapshot metadata")
                    .ino(),
            );
        }
        // An edit to an older immutable generation must not block unrelated
        // source imports or a new snapshot generation; neither overwrites it.
        fs::write(snapshot.join("tracked.txt"), b"user annotation\n").unwrap();
        let imported = materialize_source_markdown(
            &companion_dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "issue-hash",
            b"issue body\n",
        )
        .unwrap();
        assert_eq!(
            fs::read(companion.join(imported.0)).unwrap(),
            b"issue body\n"
        );
        let next = snapshot_working_tree(
            &configuration(),
            "companion-a",
            &companion_dir,
            &companion,
            &candidate(&source),
        )
        .await
        .unwrap();
        assert_ne!(response.snapshot_path, next.snapshot_path);
        assert_eq!(
            fs::read(snapshot.join("tracked.txt")).unwrap(),
            b"user annotation\n"
        );
        assert_eq!(
            fs::read(companion.join(next.snapshot_path).join("tracked.txt")).unwrap(),
            b"dirty\n"
        );
        fs::remove_dir_all(source).expect("cleanup source");
        fs::remove_dir_all(companion).expect("cleanup companion");
    }

    #[tokio::test]
    async fn concurrent_snapshots_preserve_both_manifest_generations() {
        let source = temp_dir("concurrent-repository");
        let companion = temp_dir("concurrent-companion");
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.email", "test@example.invalid"]);
        git(&source, &["config", "user.name", "Cockpit test"]);
        fs::write(source.join("tracked.txt"), b"snapshot\n").expect("tracked");
        git(&source, &["add", "tracked.txt"]);
        git(&source, &["commit", "--quiet", "-m", "initial"]);
        let companion_dir = Dir::open_ambient_dir(&companion, cap_std::ambient_authority())
            .expect("open companion");
        let association = association("workspace-a", &source);
        atomic_write_json(&companion_dir, "manifest.json", &association).expect("association");
        let configuration = configuration();
        let repository = candidate(&source);
        let first = snapshot_working_tree(
            &configuration,
            "companion-a",
            &companion_dir,
            &companion,
            &repository,
        );
        let second = snapshot_working_tree(
            &configuration,
            "companion-a",
            &companion_dir,
            &companion,
            &repository,
        );
        let (first, second) = tokio::join!(first, second);
        let first = first.expect("first snapshot");
        let second = second.expect("second snapshot");
        assert_ne!(first.generation, second.generation);
        let manifest =
            read_manifest(&companion_dir, "companion-a", &association).expect("manifest");
        assert_eq!(
            manifest
                .entries
                .iter()
                .filter(|entry| entry.status == "complete")
                .count(),
            2
        );
        fs::remove_dir_all(source).expect("cleanup source");
        fs::remove_dir_all(companion).expect("cleanup companion");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_are_not_eligible_sources() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("links");
        fs::write(root.join("source"), b"bytes").expect("source");
        symlink("source", root.join("link")).expect("symlink");
        fs::hard_link(root.join("source"), root.join("alias")).expect("hardlink");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        assert_eq!(
            read_stable_source(&root_dir, Path::new("link"))
                .expect_err("symlink")
                .code,
            "context_snapshot_symlink"
        );
        assert_eq!(
            read_stable_source(&root_dir, Path::new("alias"))
                .expect_err("hardlink")
                .code,
            "context_snapshot_hardlink"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn generated_source_refresh_preserves_user_edits_and_replaces_owned_bytes() {
        let root = temp_dir("source-refresh");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        atomic_write_json(
            &dir,
            "manifest.json",
            &association("workspace-a", Path::new("/worktree-a")),
        )
        .expect("association");
        let first = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "one",
            b"first\n",
        )
        .expect("first");
        assert!(first.1);
        let second = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("2"),
            "two",
            b"second\n",
        )
        .expect("replace owned");
        assert!(second.1);
        assert_eq!(
            fs::read(root.join(&second.0)).expect("replaced"),
            b"second\n"
        );
        fs::write(root.join(&second.0), b"user edit\n").expect("edit");
        assert_eq!(
            materialize_source_markdown(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#1",
                Some("3"),
                "three",
                b"third\n"
            )
            .expect_err("conflict")
            .code,
            "source_sync_conflict"
        );
        assert_eq!(
            fs::read(root.join(&second.0)).expect("preserved"),
            b"user edit\n"
        );
        let conflict = source_materialization_state(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            "three",
        )
        .expect("conflict state");
        assert_eq!(conflict.0, SourceFreshness::Conflict);
        assert_eq!(conflict.2.as_deref(), Some(second.0.as_str()));
        let absent = source_materialization_state(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#2",
            "other",
        )
        .expect("other source");
        assert!(absent.2.is_none());
        fs::remove_file(root.join(&second.0)).expect("remove generated file");
        let missing = source_materialization_state(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            "three",
        )
        .expect("missing state");
        assert_eq!(missing.0, SourceFreshness::Unavailable);
        assert_eq!(missing.2.as_deref(), Some(second.0.as_str()));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn interrupted_source_refresh_commits_the_matching_pending_publish() {
        let root = temp_dir("source-publish-recovery");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let association = association("workspace-a", Path::new("/worktree-a"));
        atomic_write_json(&dir, "manifest.json", &association).expect("association");
        let first = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "one",
            b"first\n",
        )
        .expect("first source");
        let mut manifest = read_manifest(&dir, "companion-a", &association).expect("manifest");
        let previous = manifest.entries[0].clone();
        let intended = stage_interrupted_source_refresh(&dir, &mut manifest, previous, b"second\n");
        // This models a process crash after the source rename but before the
        // final manifest replacement. The pending intent was written first.
        rename_source_for_interrupted_publish(&dir, &first.0, b"second\n");

        let recovered = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("2"),
            "two",
            b"second\n",
        )
        .expect("recover publish");
        assert_eq!(recovered, (first.0.clone(), false));
        let recovered_manifest =
            read_manifest(&dir, "companion-a", &association).expect("recovered manifest");
        assert_eq!(recovered_manifest.entries, vec![intended]);
        assert!(recovered_manifest.pending_source_intent.is_none());
        assert_eq!(
            source_materialization_state(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#1",
                "two",
            )
            .expect("fresh state")
            .0,
            SourceFreshness::Fresh
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn interrupted_source_refresh_does_not_adopt_user_bytes() {
        let root = temp_dir("source-publish-conflict");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let association = association("workspace-a", Path::new("/worktree-a"));
        atomic_write_json(&dir, "manifest.json", &association).expect("association");
        let first = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "one",
            b"first\n",
        )
        .expect("first source");
        let mut manifest = read_manifest(&dir, "companion-a", &association).expect("manifest");
        let previous = manifest.entries[0].clone();
        stage_interrupted_source_refresh(&dir, &mut manifest, previous.clone(), b"second\n");
        fs::write(root.join(&first.0), b"user edit\n").expect("user edit");

        assert_eq!(
            materialize_source_markdown(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#1",
                Some("2"),
                "two",
                b"second\n",
            )
            .expect_err("conflict")
            .code,
            "source_sync_conflict"
        );
        assert_eq!(
            fs::read(root.join(&first.0)).expect("preserved"),
            b"user edit\n"
        );
        let preserved = read_manifest(&dir, "companion-a", &association).expect("manifest");
        assert_eq!(preserved.entries, vec![previous]);
        assert!(preserved.pending_source_intent.is_none());
        assert!(
            materialize_source_markdown(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#2",
                Some("1"),
                "other",
                b"unrelated\n",
            )
            .expect("unrelated source remains importable")
            .1
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
