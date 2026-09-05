use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use cockpit_protocol::projects::{
    ProjectArtifact, ProjectConfiguration, ProjectDiagnostic, RepositoryCandidate,
    RepositoryListResponse,
};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use url::Url;

use crate::InspectionError;

const MAX_INPUT_BYTES: usize = 256;

#[derive(Clone)]
pub struct RepositoryCatalog {
    config: ProjectConfiguration,
}

impl RepositoryCatalog {
    pub fn new(config: ProjectConfiguration) -> Self {
        Self { config }
    }

    pub async fn list(&self) -> Result<RepositoryListResponse, InspectionError> {
        let deadline =
            Instant::now() + Duration::from_millis(self.config.limits.operation_timeout_ms as u64);
        let mut entries_seen = 0u32;
        let mut repositories = Vec::new();
        let mut diagnostics = Vec::new();
        let mut roots = BTreeSet::new();
        for configured in &self.config.repository_roots {
            if Instant::now() >= deadline {
                diagnostics.push(diagnostic(
                    "catalog_timeout",
                    "repository discovery time limit reached",
                    Some(configured),
                ));
                break;
            }
            let configured_path = PathBuf::from(configured);
            let canonical = match fs::canonicalize(&configured_path) {
                Ok(path) if path.is_dir() => path,
                Ok(_) => {
                    diagnostics.push(diagnostic(
                        "repository_root_unavailable",
                        "configured repository root is not a directory",
                        Some(configured),
                    ));
                    continue;
                }
                Err(error) => {
                    diagnostics.push(diagnostic("repository_root_unavailable", &format!("cannot access configured repository root: {error}; choose an existing directory"), Some(configured)));
                    continue;
                }
            };
            if !roots.insert(canonical.clone()) {
                continue;
            }
            let mut pending = vec![(canonical.clone(), 0u32)];
            while let Some((directory, depth)) = pending.pop() {
                if Instant::now() >= deadline {
                    diagnostics.push(diagnostic(
                        "catalog_timeout",
                        "repository discovery time limit reached",
                        Some(configured),
                    ));
                    break;
                }
                if entries_seen >= self.config.limits.catalog_entries {
                    diagnostics.push(diagnostic(
                        "catalog_entries_bounded",
                        "repository discovery entry limit reached",
                        Some(configured),
                    ));
                    break;
                }
                entries_seen += 1;
                if has_git_metadata(&directory) {
                    match self.inspect_checkout(&directory).await {
                        Ok(candidate)
                            if is_within(Path::new(&candidate.checkout_path), &canonical)
                                && is_within(Path::new(&candidate.root), &canonical) =>
                        {
                            repositories.push(candidate)
                        }
                        Ok(candidate) => diagnostics.push(diagnostic(
                            "repository_root_escape",
                            "Git checkout resolves outside the configured repository root",
                            Some(&candidate.root),
                        )),
                        Err(error) => diagnostics.push(diagnostic(
                            &error.code,
                            &error.message,
                            Some(&directory.to_string_lossy()),
                        )),
                    }
                }
                if depth >= self.config.limits.catalog_depth {
                    continue;
                }
                let Ok(read_dir) = fs::read_dir(&directory) else {
                    diagnostics.push(diagnostic(
                        "repository_root_unavailable",
                        "cannot read a configured repository directory",
                        Some(&directory.to_string_lossy()),
                    ));
                    continue;
                };
                let mut children = Vec::new();
                let mut entry_limit = false;
                let mut timed_out = false;
                for entry in read_dir {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        break;
                    }
                    if entries_seen >= self.config.limits.catalog_entries {
                        entry_limit = true;
                        break;
                    }
                    entries_seen += 1;
                    let Ok(entry) = entry else { continue };
                    let path = entry.path();
                    let Ok(metadata) = fs::symlink_metadata(&path) else {
                        continue;
                    };
                    if metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && entry.file_name() != ".git"
                    {
                        children.push(path);
                    }
                }
                if timed_out {
                    diagnostics.push(diagnostic(
                        "catalog_timeout",
                        "repository discovery time limit reached",
                        Some(configured),
                    ));
                }
                if entry_limit {
                    diagnostics.push(diagnostic(
                        "catalog_entries_bounded",
                        "repository discovery entry limit reached",
                        Some(configured),
                    ));
                }
                children.sort_by(|left, right| right.cmp(left));
                pending.extend(children.into_iter().map(|child| (child, depth + 1)));
            }
        }
        repositories.sort_by(|left, right| left.repository_id.cmp(&right.repository_id));
        repositories.dedup_by(|left, right| left.repository_id == right.repository_id);
        Ok(RepositoryListResponse {
            repositories,
            diagnostics,
        })
    }

    pub async fn resolve(
        &self,
        repository_id: &str,
    ) -> Result<RepositoryCandidate, InspectionError> {
        validate_input(repository_id, "repository_id")?;
        let matches: Vec<_> = self
            .list()
            .await?
            .repositories
            .into_iter()
            .filter(|candidate| candidate.repository_id == repository_id)
            .collect();
        match matches.as_slice() {
            [candidate] => Ok(candidate.clone()),
            [] => Err(InspectionError::new(
                "repository_not_found",
                "repository identity was not found beneath configured roots; rescan and select an available repository",
            )),
            _ => Err(InspectionError::new(
                "repository_identity_conflict",
                "repository identity matched multiple checkouts; require explicit selection",
            )),
        }
    }

    pub async fn validate_branch(
        &self,
        candidate: &RepositoryCandidate,
        branch: &str,
    ) -> Result<(), InspectionError> {
        let fresh = self.fresh_candidate(candidate).await?;
        validate_input(branch, "branch")?;
        if branch.starts_with('-') || branch.chars().any(char::is_whitespace) {
            return Err(InspectionError::new(
                "invalid_branch",
                "branch must be a bounded Git branch name",
            ));
        }
        let output = self
            .git_output(
                Path::new(&fresh.checkout_path),
                &["check-ref-format", "--branch", branch],
            )
            .await?;
        if output.status.success() {
            Ok(())
        } else {
            Err(InspectionError::new(
                "invalid_branch",
                format!("Git rejected branch name {branch:?}"),
            ))
        }
    }

    pub async fn resolve_base(
        &self,
        candidate: &RepositoryCandidate,
        base: &str,
    ) -> Result<String, InspectionError> {
        let fresh = self.fresh_candidate(candidate).await?;
        validate_input(base, "base")?;
        if base.starts_with('-') || base.chars().any(char::is_whitespace) {
            return Err(InspectionError::new(
                "invalid_base",
                "base must be a bounded Git revision",
            ));
        }
        let revision = format!("{base}^{{commit}}");
        let output = self
            .git_output(
                Path::new(&fresh.checkout_path),
                &["rev-parse", "--verify", "--end-of-options", &revision],
            )
            .await?;
        if !output.status.success() {
            return Err(InspectionError::new(
                "invalid_base",
                format!("Git could not resolve immutable commit {base:?}"),
            ));
        }
        let commit = stdout_text(&output)?;
        if commit.is_empty() || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InspectionError::new(
                "invalid_base",
                "Git returned an invalid commit identity",
            ));
        }
        Ok(commit)
    }

    async fn fresh_candidate(
        &self,
        candidate: &RepositoryCandidate,
    ) -> Result<RepositoryCandidate, InspectionError> {
        let fresh = self.resolve(&candidate.repository_id).await?;
        if fresh.root != candidate.root
            || fresh.common_dir != candidate.common_dir
            || fresh.checkout_path != candidate.checkout_path
        {
            return Err(InspectionError::new(
                "repository_identity_stale",
                "repository changed since selection; select it again",
            ));
        }
        Ok(fresh)
    }

    async fn inspect_checkout(
        &self,
        directory: &Path,
    ) -> Result<RepositoryCandidate, InspectionError> {
        let checkout = fs::canonicalize(directory).map_err(|error| {
            InspectionError::new(
                "repository_unavailable",
                format!("cannot canonicalize checkout: {error}"),
            )
        })?;
        let git_root = fs::canonicalize(
            self.git_text(&checkout, &["rev-parse", "--show-toplevel"])
                .await?
                .trim(),
        )
        .map_err(|error| {
            InspectionError::new(
                "repository_unavailable",
                format!("cannot canonicalize Git root: {error}"),
            )
        })?;
        let common_raw = PathBuf::from(
            self.git_text(&checkout, &["rev-parse", "--git-common-dir"])
                .await?
                .trim(),
        );
        let common = fs::canonicalize(if common_raw.is_absolute() {
            common_raw
        } else {
            checkout.join(common_raw)
        })
        .map_err(|error| {
            InspectionError::new(
                "repository_unavailable",
                format!("cannot canonicalize Git common directory: {error}"),
            )
        })?;
        let root = common.parent().map(Path::to_path_buf).unwrap_or(git_root);
        let linked = fs::symlink_metadata(checkout.join(".git"))
            .map(|metadata| metadata.is_file())
            .unwrap_or(false);
        let branch_output = self
            .git_output(&checkout, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await?;
        let branch = branch_output
            .status
            .success()
            .then(|| stdout_lossy(&branch_output).trim().to_owned())
            .filter(|value| !value.is_empty());
        let head_output = self
            .git_output(&checkout, &["rev-parse", "--verify", "HEAD"])
            .await?;
        let detached = branch.is_none() && head_output.status.success();
        let unborn = branch.is_some() && !head_output.status.success();
        let id = repository_identity(&root, &common, &checkout)?;
        let mut provenance = format!(
            "{};root={};common={}",
            if linked { "linked" } else { "primary" },
            root.display(),
            common.display()
        );
        if detached {
            provenance.push_str(";detached");
        }
        if unborn {
            provenance.push_str(";unborn");
        }
        let name = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("repository")
            .to_owned();
        Ok(RepositoryCandidate {
            repository_id: id,
            name,
            root: root.to_string_lossy().into_owned(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            common_dir: common.to_string_lossy().into_owned(),
            branch,
            is_linked_worktree: linked,
            is_detached: detached,
            provenance,
        })
    }

    async fn git_text(&self, directory: &Path, args: &[&str]) -> Result<String, InspectionError> {
        let output = self.git_output(directory, args).await?;
        if !output.status.success() {
            return Err(InspectionError::new(
                "repository_unavailable",
                format!("Git command {:?} failed: {}", args, stderr_lossy(&output)),
            ));
        }
        stdout_text(&output)
    }

    async fn git_output(&self, directory: &Path, args: &[&str]) -> Result<Output, InspectionError> {
        let mut command = Command::new("git");
        command
            .current_dir(directory)
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
            .env("GIT_ASKPASS", "");
        crate::process::run_bounded_command(
            command,
            self.config.limits.git_output_bytes as usize,
            self.config.limits.git_output_bytes as usize,
            Duration::from_millis(self.config.limits.git_timeout_ms as u64),
            "git",
        )
        .await
    }
}

pub fn resolve_artifact(
    config: &ProjectConfiguration,
    original_url: &str,
) -> Result<ProjectArtifact, InspectionError> {
    if original_url.is_empty()
        || original_url.len() > 8192
        || original_url.chars().any(|character| character.is_control())
    {
        return Err(InspectionError::new(
            "invalid_artifact_url",
            "artifact URL must be a bounded absolute HTTP(S) URL",
        ));
    }
    let parsed = Url::parse(original_url).map_err(|_| {
        InspectionError::new(
            "invalid_artifact_url",
            "artifact URL must be an absolute HTTP(S) URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(InspectionError::new(
            "invalid_artifact_url",
            "artifact URL must be credential-free HTTP(S)",
        ));
    }
    let provider = config
        .providers
        .iter()
        .find(|provider| provider_matches(&parsed, &provider.base_url))
        .ok_or_else(|| {
            InspectionError::new(
                "unsupported_artifact",
                "artifact host is not a configured Gitea provider",
            )
        })?;
    let base = Url::parse(&provider.base_url).map_err(|_| {
        InspectionError::new(
            "invalid_provider_base_url",
            "configured provider URL is invalid",
        )
    })?;
    let relative = parsed
        .path()
        .strip_prefix(base.path().trim_end_matches('/'))
        .unwrap_or(parsed.path())
        .trim_matches('/');
    let pieces: Vec<&str> = relative.split('/').collect();
    if pieces.len() < 4
        || pieces[..4]
            .iter()
            .any(|piece| piece.is_empty() || *piece == "." || *piece == ".." || piece.contains('%'))
    {
        return Err(InspectionError::new(
            "unsupported_artifact",
            "Gitea artifact path must identify owner, repository, kind, and id",
        ));
    }
    let repository = format!("{}/{}", pieces[0], pieces[1]);
    let (kind, canonical_id) = match pieces[2] {
        "issues" if pieces.len() == 4 && pieces[3].bytes().all(|byte| byte.is_ascii_digit()) => {
            ("issue", format!("{repository}#{}", pieces[3]))
        }
        "pulls" | "pull"
            if pieces.len() == 4 && pieces[3].bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            ("review", format!("{repository}!{}", pieces[3]))
        }
        "wiki"
            if pieces[3..]
                .iter()
                .all(|piece| !piece.is_empty() && *piece != "." && *piece != "..") =>
        {
            (
                "wiki",
                format!("{repository}/wiki/{}", pieces[3..].join("/")),
            )
        }
        _ => {
            return Err(InspectionError::new(
                "unsupported_artifact",
                "unsupported Gitea artifact kind or malformed id",
            ));
        }
    };
    Ok(ProjectArtifact {
        provider_id: provider.id.clone(),
        kind: kind.into(),
        canonical_id,
        original_url: original_url.into(),
        canonical_url: parsed.to_string(),
    })
}

fn provider_matches(url: &Url, configured: &str) -> bool {
    let Ok(base) = Url::parse(configured) else {
        return false;
    };
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return false;
    }
    let base_path = base.path().trim_end_matches('/');
    let path_matches = base_path.is_empty()
        || url.path() == base_path
        || url.path().starts_with(&format!("{base_path}/"));
    url.scheme() == base.scheme()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default()
        && path_matches
}
fn has_git_metadata(directory: &Path) -> bool {
    fs::symlink_metadata(directory.join(".git"))
        .map(|metadata| metadata.is_dir() || metadata.is_file())
        .unwrap_or(false)
}
fn is_within(path: &Path, root: &Path) -> bool {
    path == root || path.strip_prefix(root).is_ok()
}
fn validate_input(value: &str, field: &str) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > MAX_INPUT_BYTES
        || value.chars().any(|character| character.is_control())
    {
        Err(InspectionError::new(
            format!("invalid_{field}"),
            format!(
                "{field} must be nonempty printable text no longer than {MAX_INPUT_BYTES} bytes"
            ),
        ))
    } else {
        Ok(())
    }
}
fn repository_identity(
    root: &Path,
    common: &Path,
    checkout: &Path,
) -> Result<String, InspectionError> {
    let mut hasher = Sha256::new();
    for path in [root, common, checkout] {
        hasher.update(path.to_string_lossy().as_bytes());
        let metadata = fs::metadata(path).map_err(|error| {
            InspectionError::new(
                "repository_unavailable",
                format!("cannot inspect repository identity: {error}"),
            )
        })?;
        hasher.update(metadata_identity(&metadata).as_bytes());
        hasher.update([0]);
    }
    Ok(format!(
        "repo-{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}
fn metadata_identity(metadata: &fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}:{}", metadata.dev(), metadata.ino(), metadata.mode())
    }
    #[cfg(not(unix))]
    {
        format!("{}:{:?}", metadata.len(), metadata.modified().ok())
    }
}
fn stdout_text(output: &Output) -> Result<String, InspectionError> {
    String::from_utf8(output.stdout.clone())
        .map(|value| value.trim().to_owned())
        .map_err(|_| InspectionError::new("git_output_invalid", "Git returned non-UTF-8 output"))
}
fn stdout_lossy(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
fn stderr_lossy(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}
fn diagnostic(code: &str, message: &str, path: Option<&str>) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.into(),
        message: message.into(),
        path: path.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_artifact;
    use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
    use std::collections::BTreeMap;

    fn config() -> ProjectConfiguration {
        ProjectConfiguration {
            version: 1,
            repository_roots: vec![".".into()],
            worktree_root: "worktrees".into(),
            companion_root: "companions".into(),
            state_root: "state".into(),
            branch_template: "{repo}/{task_id}".into(),
            checkout_template: "{repo}-{task_id}".into(),
            providers: vec![ProjectProvider {
                id: "gitea".into(),
                base_url: "https://git.example.test/".into(),
                executable: "tea".into(),
            }],
            limits: ProjectLimits {
                catalog_depth: 3,
                catalog_entries: 100,
                git_timeout_ms: 1000,
                git_output_bytes: 1024,
                operation_timeout_ms: 1000,
                context_preview_bytes: 1024 * 1024,
                context_preview_lines: 5000,
                context_directory_entries: 1000,
                context_tree_depth: 32,
            },
            origins: BTreeMap::new(),
        }
    }

    #[test]
    fn artifact_requires_configured_credential_free_provider() {
        let config = config();
        let artifact = resolve_artifact(&config, "https://git.example.test/acme/app/issues/42")
            .expect("valid issue URL");
        assert_eq!(artifact.kind, "issue");
        assert_eq!(artifact.canonical_id, "acme/app#42");
        assert!(
            resolve_artifact(&config, "https://other.example.test/acme/app/issues/42").is_err()
        );
        assert!(
            resolve_artifact(
                &config,
                "https://user:secret@git.example.test/acme/app/issues/42"
            )
            .is_err()
        );
        assert!(
            resolve_artifact(
                &config,
                "https://git.example.test/acme/app/issues/not-a-number"
            )
            .is_err()
        );
    }
}
