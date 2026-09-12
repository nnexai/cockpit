use std::collections::HashSet;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::OpenOptions;
use cockpit_protocol::context_search::{
    ContextInvalidation, ContextInvalidationRequest, ContextInvalidationResponse,
    ContextInvalidationState, ContextSearchRequest, ContextSearchResponse, ContextSearchResult,
};

use crate::context::{metadata_revision, AuthorizedRoot, ContextService};
use crate::InspectionError;

const MAX_QUERY_BYTES: usize = 256;
const MAX_RESULTS: usize = 1_000;
const MAX_SCANNED_ENTRIES: usize = 100_000;
const MAX_SEARCHED_FILE_BYTES: usize = 1024 * 1024;
const MAX_EXCERPT_BYTES: usize = 512;
const MAX_KNOWN_REVISIONS: usize = 128;
const SEARCH_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Clone)]
pub struct ContextSearchService {
    context: Arc<ContextService>,
}

impl ContextSearchService {
    pub fn new(context: Arc<ContextService>) -> Self {
        Self { context }
    }

    pub async fn search(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextSearchRequest,
    ) -> Result<ContextSearchResponse, InspectionError> {
        validate_search_request(request)?;
        let root = self
            .context
            .authorize_companion_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let request = request.clone();
        let permit = self
            .context
            .search_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                InspectionError::new(
                    "context_search_unavailable",
                    "Context search capacity is unavailable",
                )
            })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            search_blocking(root, request)
        })
        .await
        .map_err(|error| InspectionError::new("context_search_task", error.to_string()))?
    }

    pub async fn invalidate(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextInvalidationRequest,
    ) -> Result<ContextInvalidationResponse, InspectionError> {
        validate_invalidation_request(request)?;
        let root = self
            .context
            .authorize_companion_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let request = request.clone();
        let permit = self
            .context
            .search_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                InspectionError::new(
                    "context_invalidation_unavailable",
                    "Context invalidation capacity is unavailable",
                )
            })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            invalidate_blocking(root, request)
        })
        .await
        .map_err(|error| InspectionError::new("context_invalidation_task", error.to_string()))?
    }
}

fn validate_search_request(request: &ContextSearchRequest) -> Result<(), InspectionError> {
    if request.request_generation == 0
        || request.query.is_empty()
        || request.query.len() > MAX_QUERY_BYTES
        || request.query.chars().any(char::is_control)
    {
        return Err(InspectionError::new(
            "context_search_invalid",
            "Context search requires a bounded single-line query and generation",
        ));
    }
    Ok(())
}

fn validate_invalidation_request(
    request: &ContextInvalidationRequest,
) -> Result<(), InspectionError> {
    if request.request_generation == 0 || request.known.len() > MAX_KNOWN_REVISIONS {
        return Err(InspectionError::new(
            "context_invalidation_bounded",
            "Context invalidation request exceeds its bounded generation or file count",
        ));
    }
    let mut paths = HashSet::with_capacity(request.known.len());
    for known in &request.known {
        if known.revision.is_empty() || known.revision.len() > 4096 || !paths.insert(&known.path) {
            return Err(InspectionError::new(
                "context_invalidation_invalid",
                "Context invalidation paths and revisions must be unique and bounded",
            ));
        }
    }
    Ok(())
}

fn search_blocking(
    root: AuthorizedRoot,
    request: ContextSearchRequest,
) -> Result<ContextSearchResponse, InspectionError> {
    root.revalidate()?;
    let deadline = Instant::now() + SEARCH_TIMEOUT;
    let mut pending = vec![(root.resolve_directory(Path::new(""))?, PathBuf::new())];
    let mut results = Vec::new();
    let mut scanned_entries = 0usize;
    let mut scanned_files = 0u32;
    let mut truncated = false;

    while let Some((directory, relative)) = pending.pop() {
        if Instant::now() >= deadline || scanned_entries >= MAX_SCANNED_ENTRIES {
            truncated = true;
            break;
        }
        let mut names = Vec::new();
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let entries = directory.entries().map_err(|error| {
            InspectionError::new("context_search_unavailable", error.to_string())
        })?;
        for entry in entries {
            if Instant::now() >= deadline || scanned_entries >= MAX_SCANNED_ENTRIES {
                truncated = true;
                break;
            }
            scanned_entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    truncated = true;
                    continue;
                }
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                truncated = true;
                continue;
            };
            names.push(name);
        }
        names.sort();

        for name in names {
            if Instant::now() >= deadline || results.len() >= MAX_RESULTS {
                truncated = true;
                break;
            }
            let candidate = if relative.as_os_str().is_empty() {
                PathBuf::from(&name)
            } else {
                relative.join(&name)
            };
            let candidate = match root.relative_path(&candidate.to_string_lossy()) {
                Ok(candidate) => candidate,
                Err(_) => continue,
            };
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
            let metadata = match directory.symlink_metadata(Path::new(&name)) {
                Ok(metadata) => metadata,
                Err(_) => {
                    truncated = true;
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                match directory.open_dir_nofollow(Path::new(&name)) {
                    Ok(child) => pending.push((child, candidate)),
                    Err(_) => truncated = true,
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if metadata.len() > MAX_SEARCHED_FILE_BYTES as u64 {
                truncated = true;
                continue;
            }
            let (text, revision) =
                match read_utf8_file(&root, &candidate, MAX_SEARCHED_FILE_BYTES, deadline) {
                    Ok(Some(value)) => value,
                    Ok(None) => continue,
                    Err(error) if error.code == "context_search_deadline" => {
                        truncated = true;
                        break;
                    }
                    Err(error) => return Err(error),
                };
            scanned_files = scanned_files.saturating_add(1);
            append_matches(
                &mut results,
                &candidate.to_string_lossy(),
                &revision,
                &text,
                &request.query,
            );
            if results.len() >= MAX_RESULTS {
                truncated = true;
                break;
            }
        }
    }
    if Instant::now() >= deadline {
        truncated = true;
    } else {
        root.revalidate()?;
    }
    Ok(ContextSearchResponse {
        binding_id: request.binding_id,
        root_id: root.root_id().to_owned(),
        query: request.query,
        request_generation: request.request_generation,
        results,
        scanned_files,
        truncated,
    })
}

fn invalidate_blocking(
    root: AuthorizedRoot,
    request: ContextInvalidationRequest,
) -> Result<ContextInvalidationResponse, InspectionError> {
    root.revalidate()?;
    let deadline = Instant::now() + SEARCH_TIMEOUT;
    let mut invalidations = Vec::new();
    for known in &request.known {
        if Instant::now() >= deadline {
            return Ok(ContextInvalidationResponse {
                binding_id: request.binding_id,
                root_id: root.root_id().to_owned(),
                request_generation: request.request_generation,
                invalidations,
                truncated: true,
            });
        }
        let relative = root.relative_path(&known.path)?;
        let state = match file_revision(&root, &relative, deadline) {
            Ok(Some(revision)) if revision == known.revision => None,
            Ok(Some(revision)) => Some((ContextInvalidationState::Changed, Some(revision))),
            Ok(None) => Some((ContextInvalidationState::Unavailable, None)),
            Err(error) if error.code == "context_file_missing" => {
                Some((ContextInvalidationState::Missing, None))
            }
            Err(error) if error.code == "context_search_deadline" => {
                return Ok(ContextInvalidationResponse {
                    binding_id: request.binding_id,
                    root_id: root.root_id().to_owned(),
                    request_generation: request.request_generation,
                    invalidations,
                    truncated: true,
                });
            }
            Err(_) => Some((ContextInvalidationState::Unavailable, None)),
        };
        if let Some((state, revision)) = state {
            invalidations.push(ContextInvalidation {
                path: known.path.clone(),
                state,
                revision,
            });
        }
    }
    if Instant::now() >= deadline {
        return Ok(ContextInvalidationResponse {
            binding_id: request.binding_id,
            root_id: root.root_id().to_owned(),
            request_generation: request.request_generation,
            invalidations,
            truncated: true,
        });
    }
    root.revalidate()?;
    Ok(ContextInvalidationResponse {
        binding_id: request.binding_id,
        root_id: root.root_id().to_owned(),
        request_generation: request.request_generation,
        invalidations,
        truncated: false,
    })
}

fn file_revision(
    root: &AuthorizedRoot,
    relative: &Path,
    deadline: Instant,
) -> Result<Option<String>, InspectionError> {
    check_deadline(deadline)?;
    let (parent, leaf) = root.resolve_parent(relative)?;
    check_deadline(deadline)?;
    let metadata = parent.symlink_metadata(&leaf).map_err(|error| {
        InspectionError::new(
            if error.kind() == ErrorKind::NotFound {
                "context_file_missing"
            } else {
                "context_file_unavailable"
            },
            format!("cannot inspect context file: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    Ok(Some(metadata_revision(&metadata)))
}

fn read_utf8_file(
    root: &AuthorizedRoot,
    relative: &Path,
    max_bytes: usize,
    deadline: Instant,
) -> Result<Option<(String, String)>, InspectionError> {
    check_deadline(deadline)?;
    let (parent, leaf) = root.resolve_parent(relative)?;
    check_deadline(deadline)?;
    let before = parent.symlink_metadata(&leaf).map_err(|error| {
        InspectionError::new(
            if error.kind() == ErrorKind::NotFound {
                "context_file_missing"
            } else {
                "context_file_unavailable"
            },
            format!("cannot inspect context file: {error}"),
        )
    })?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > max_bytes as u64 {
        return Ok(None);
    }
    let revision = metadata_revision(&before);
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    check_deadline(deadline)?;
    let file = match parent.open_with(&leaf, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(InspectionError::new(
                "context_file_missing",
                error.to_string(),
            ));
        }
        Err(_) => return Ok(None),
    };
    check_deadline(deadline)?;
    let opened = match file.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return Ok(None),
    };
    if opened.file_type().is_symlink()
        || !opened.is_file()
        || metadata_revision(&opened) != revision
    {
        return Ok(None);
    }
    let mut content = Vec::with_capacity(before.len() as usize);
    check_deadline(deadline)?;
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut content)
        .map_err(|error| InspectionError::new("context_file_unavailable", error.to_string()))?;
    check_deadline(deadline)?;
    let after = parent
        .symlink_metadata(&leaf)
        .map_err(|error| InspectionError::new("context_changed_during_read", error.to_string()))?;
    if metadata_revision(&after) != revision || content.len() > max_bytes {
        return Ok(None);
    }
    if content.contains(&0) {
        return Ok(None);
    }
    let text = match String::from_utf8(content) {
        Ok(text) => text,
        Err(_) => return Ok(None),
    };
    Ok(Some((text, revision)))
}

fn check_deadline(deadline: Instant) -> Result<(), InspectionError> {
    if Instant::now() >= deadline {
        return Err(InspectionError::new(
            "context_search_deadline",
            "bounded Context search reached its deadline",
        ));
    }
    Ok(())
}

fn append_matches(
    results: &mut Vec<ContextSearchResult>,
    path: &str,
    revision: &str,
    text: &str,
    query: &str,
) {
    for (index, raw_line) in text.split_inclusive('\n').enumerate() {
        if results.len() >= MAX_RESULTS || !raw_line.contains(query) {
            continue;
        }
        let line = raw_line
            .strip_suffix('\n')
            .and_then(|line| line.strip_suffix('\r').or(Some(line)))
            .unwrap_or(raw_line);
        results.push(ContextSearchResult {
            path: path.to_owned(),
            line: (index + 1) as u32,
            excerpt: truncate_utf8(line, MAX_EXCERPT_BYTES),
            revision: revision.to_owned(),
        });
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes.saturating_sub('…'.len_utf8());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::{
        append_matches, truncate_utf8, validate_invalidation_request, validate_search_request,
    };
    use cockpit_protocol::context_search::{
        ContextInvalidationRequest, ContextKnownRevision, ContextSearchRequest,
    };

    #[test]
    fn search_preserves_lf_line_numbers_and_lone_carriage_returns() {
        let mut results = Vec::new();
        append_matches(
            &mut results,
            "notes.md",
            "revision",
            "first\rneedle\r\nsecond needle\nlast",
            "needle",
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].excerpt, "first\rneedle");
        assert_eq!(results[1].line, 2);
    }

    #[test]
    fn excerpts_are_bounded_at_utf8_boundaries() {
        let excerpt = truncate_utf8(&"é".repeat(300), 512);
        assert!(excerpt.len() <= 512);
        assert!(excerpt.ends_with('…'));
    }

    #[test]
    fn requests_reject_control_queries_and_unbounded_known_files() {
        let invalid_query = ContextSearchRequest {
            binding_id: "binding".to_owned(),
            root_id: "root".to_owned(),
            query: "first\nsecond".to_owned(),
            request_generation: 1,
        };
        assert!(validate_search_request(&invalid_query).is_err());
        let invalidation = ContextInvalidationRequest {
            binding_id: "binding".to_owned(),
            root_id: "root".to_owned(),
            request_generation: 1,
            known: (0..129)
                .map(|index| ContextKnownRevision {
                    path: format!("{index}.md"),
                    revision: "revision".to_owned(),
                })
                .collect(),
        };
        assert!(validate_invalidation_request(&invalidation).is_err());
    }
}
