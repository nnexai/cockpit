use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use cockpit_protocol::review::ReviewComparison;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};

use crate::InspectionError;
use crate::process::{OwnedChild, run_bounded_command};

use super::ReviewService;
use super::parse::{ChangeStatistics, parse_name_status, parse_numstat, parse_untracked};
use super::safe_fs::{open_worktree_file, open_worktree_parent, safe_relative_path};
use super::snapshot::Change;
use super::source::{FrozenSource, MAX_FILE_BYTES};

pub(super) const MAX_FILE_LIST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RevisionTokens {
    pub(super) head: Option<String>,
    pub(super) index: String,
    pub(super) worktree: String,
}

impl ReviewService {
    pub(super) async fn change_statistics(
        &self,
        checkout: &Path,
        comparison: ReviewComparison,
        base: Option<&str>,
    ) -> Result<BTreeMap<String, ChangeStatistics>, InspectionError> {
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            "-z",
            "--find-renames=50%",
        ];
        match comparison {
            ReviewComparison::Staged => args.push("--cached"),
            ReviewComparison::Branch => {
                args.push(base.expect("validated branch base"));
                args.push("HEAD");
            }
            ReviewComparison::Unstaged => {}
            ReviewComparison::Untracked => return Ok(BTreeMap::new()),
            ReviewComparison::AllLocal => unreachable!("collect expands all-local comparisons"),
        }
        args.push("--");
        let output = self
            .git_with_limit(checkout, &args, MAX_FILE_LIST_BYTES)
            .await?;
        parse_numstat(&output.stdout)
    }

    pub(super) async fn changes(
        &self,
        checkout: &Path,
        comparison: ReviewComparison,
        base: Option<&str>,
    ) -> Result<Vec<Change>, InspectionError> {
        if comparison == ReviewComparison::Untracked {
            let output = self
                .git_with_limit(
                    checkout,
                    &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                    MAX_FILE_LIST_BYTES,
                )
                .await?;
            return parse_untracked(&output.stdout);
        }
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-status",
            "-z",
            "--find-renames=50%",
        ];
        match comparison {
            ReviewComparison::Staged => args.push("--cached"),
            ReviewComparison::Branch => {
                args.push(base.expect("branch base"));
                args.push("HEAD");
            }
            ReviewComparison::Unstaged => {}
            _ => unreachable!(),
        }
        args.push("--");
        let output = self
            .git_with_limit(checkout, &args, MAX_FILE_LIST_BYTES)
            .await?;
        parse_name_status(&output.stdout, comparison)
    }

    pub(super) async fn git_source(
        &self,
        checkout: &Path,
        revision: &str,
        path: &str,
    ) -> Result<FrozenSource, InspectionError> {
        let object = if revision == ":" {
            format!(":{path}")
        } else {
            format!("{revision}:{path}")
        };
        match self
            .git_with_limit(
                checkout,
                &[
                    "show",
                    "--no-textconv",
                    "--format=",
                    "--end-of-options",
                    &object,
                ],
                MAX_FILE_BYTES,
            )
            .await
        {
            Ok(output) if output.status.success() => Ok(FrozenSource::from_bytes(output.stdout)),
            Ok(_) => Ok(FrozenSource::unavailable(
                path,
                "Git could not read the immutable source for this review side",
            )),
            Err(error) if error.code == "bounded_output" => {
                self.git_source_page(checkout, revision, path, 0).await
            }
            Err(error) => Err(error),
        }
    }

    /// Stream one bounded page from an immutable Git object. `git show` is
    /// useful for small previews, but its bounded command reader cannot seek
    /// a multi-megabyte blob without buffering it. `cat-file --batch` gives us
    /// the object size and lets this reader skip and consume only one page.
    pub(super) async fn git_source_page(
        &self,
        checkout: &Path,
        revision: &str,
        path: &str,
        offset: u32,
    ) -> Result<FrozenSource, InspectionError> {
        let object = if revision == ":" {
            format!(":{path}")
        } else {
            format!("{revision}:{path}")
        };
        let mut owned = spawn_git_source_reader(checkout)?;
        let result = tokio::time::timeout(
            Duration::from_millis(self.configuration.limits.git_timeout_ms as u64),
            async {
                let child = owned.child.as_mut().expect("owned child available");
                let (mut reader, size) = query_git_source(child, &object).await?;
                read_git_source_page(&mut reader, object, size, offset).await
            },
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => Err(InspectionError::new(
                "review_timeout",
                "Git source page exceeded the configured timeout",
            )),
        };
        let cleanup = owned.kill_and_reap().await;
        if let Some(cleanup) = cleanup {
            if result.is_ok() {
                return Err(InspectionError::new("review_task", cleanup));
            }
        }
        result
    }

    pub(super) async fn revision_tokens(
        &self,
        checkout: &Path,
    ) -> Result<RevisionTokens, InspectionError> {
        let head = self
            .git_text_optional(checkout, &["rev-parse", "--verify", "HEAD"])
            .await?;
        // `write-tree` would create an object in .git/objects. A review is
        // read-only, so retain a bounded digest of index entries instead.
        let index_digest = self
            .git_hash(checkout, &["ls-files", "--stage", "-z"], Sha256::new())
            .await?;
        let index = format!("sha256:{:x}", index_digest.finalize());
        let mut worktree_digest = Sha256::new();
        worktree_digest.update(b"cockpit-review-worktree-v3\0");
        worktree_digest = self
            .git_hash(
                checkout,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--binary",
                    "--no-color",
                    "--",
                ],
                worktree_digest,
            )
            .await?;
        let untracked = self
            .git_with_limit(
                checkout,
                &["ls-files", "--others", "--exclude-standard", "-z"],
                MAX_FILE_LIST_BYTES,
            )
            .await?;
        if !untracked.status.success() {
            return Err(InspectionError::new(
                "review_git",
                "Git could not enumerate untracked files",
            ));
        }
        update_untracked_token(checkout, &untracked.stdout, &mut worktree_digest)?;
        let worktree = format!("sha256:{:x}", worktree_digest.finalize());
        Ok(RevisionTokens {
            head,
            index,
            worktree,
        })
    }

    pub(super) async fn git_text(
        &self,
        checkout: &Path,
        args: &[&str],
    ) -> Result<String, InspectionError> {
        let output = self.git(checkout, args).await?;
        if !output.status.success() {
            return Err(InspectionError::new(
                "review_git",
                "Git could not resolve the requested immutable revision",
            ));
        }
        String::from_utf8(output.stdout)
            .map(|text| text.trim().to_owned())
            .map_err(|_| InspectionError::new("review_git", "Git emitted invalid text"))
    }

    pub(super) async fn git_text_optional(
        &self,
        checkout: &Path,
        args: &[&str],
    ) -> Result<Option<String>, InspectionError> {
        let output = self.git(checkout, args).await?;
        if !output.status.success() {
            return Ok(None);
        }
        String::from_utf8(output.stdout)
            .map(|text| Some(text.trim().to_owned()))
            .map_err(|_| InspectionError::new("review_git", "Git emitted invalid text"))
    }

    async fn git(
        &self,
        checkout: &Path,
        args: &[&str],
    ) -> Result<std::process::Output, InspectionError> {
        self.git_with_limit(
            checkout,
            args,
            self.configuration.limits.git_output_bytes as usize,
        )
        .await
    }

    pub(super) async fn git_with_limit(
        &self,
        checkout: &Path,
        args: &[&str],
        limit: usize,
    ) -> Result<std::process::Output, InspectionError> {
        let mut command = Command::new("git");
        command
            .current_dir(checkout)
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .arg("-c")
            .arg("core.fsmonitor=false")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .args(args);
        run_bounded_command(
            command,
            limit,
            limit,
            Duration::from_millis(self.configuration.limits.git_timeout_ms as u64),
            "review git",
        )
        .await
    }

    async fn git_hash(
        &self,
        checkout: &Path,
        args: &[&str],
        digest: Sha256,
    ) -> Result<Sha256, InspectionError> {
        let mut command = Command::new("git");
        command
            .current_dir(checkout)
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .arg("-c")
            .arg("core.fsmonitor=false")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .args(args);
        crate::process::run_bounded_hash_command(
            command,
            usize::MAX,
            4096,
            Duration::from_millis(self.configuration.limits.git_timeout_ms as u64),
            "review Git digest",
            digest,
        )
        .await
    }
}

#[cfg(test)]
pub(super) fn worktree_token(
    checkout: &Path,
    unstaged_diff: &[u8],
    untracked_paths: &[u8],
) -> Result<String, InspectionError> {
    let mut digest = Sha256::new();
    digest.update(b"cockpit-review-worktree-v3\0");
    digest.update(unstaged_diff);
    update_untracked_token(checkout, untracked_paths, &mut digest)?;
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn update_untracked_token(
    checkout: &Path,
    untracked_paths: &[u8],
    digest: &mut Sha256,
) -> Result<(), InspectionError> {
    // File bodies remain lazy. Metadata invalidates the token when an
    // untracked path is added, removed, resized, or rewritten.
    for raw_path in untracked_paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(raw_path).map_err(|_| {
            InspectionError::new(
                "review_path_encoding",
                "non-UTF-8 untracked path cannot form a source anchor",
            )
        })?;
        digest.update(raw_path);
        digest.update([0]);
        let (parent, leaf) = match open_worktree_parent(checkout, safe_relative_path(path)?) {
            Ok(value) => value,
            Err(error) if error.code == "review_unreadable" => {
                digest.update(b"unavailable");
                continue;
            }
            Err(error) => return Err(error),
        };
        let file = match open_worktree_file(&parent, &leaf) {
            Ok(file) => file,
            Err(_) => {
                digest.update(b"unavailable");
                continue;
            }
        };
        let metadata = file.metadata().map_err(|_| {
            InspectionError::new(
                "review_unreadable",
                "untracked source metadata is unavailable",
            )
        })?;
        if !metadata.is_file() {
            digest.update(b"nonregular");
            continue;
        }
        digest.update(b"regular");
        digest.update(metadata.len().to_le_bytes());
        digest.update(format!("{:?}", metadata.modified()).as_bytes());
        digest.update(cap_fs_ext::MetadataExt::dev(&metadata).to_le_bytes());
        digest.update(cap_fs_ext::MetadataExt::ino(&metadata).to_le_bytes());
        digest.update([0]);
    }
    Ok(())
}

fn spawn_git_source_reader(checkout: &Path) -> Result<OwnedChild, InspectionError> {
    let mut command = Command::new("git");
    command
        .current_dir(checkout)
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .args(["cat-file", "--batch"]);
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| InspectionError::new("execution_failed", "Git could not be started"))?;
    Ok(OwnedChild::new(child))
}

async fn query_git_source(
    child: &mut Child,
    object: &str,
) -> Result<(BufReader<ChildStdout>, u64), InspectionError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| InspectionError::new("execution_failed", "Git stdin unavailable"))?;
    stdin
        .write_all(object.as_bytes())
        .await
        .map_err(|_| InspectionError::new("review_source_unreadable", "Git query failed"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|_| InspectionError::new("review_source_unreadable", "Git query failed"))?;
    stdin
        .shutdown()
        .await
        .map_err(|_| InspectionError::new("review_source_unreadable", "Git query failed"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| InspectionError::new("execution_failed", "Git stdout unavailable"))?;
    let mut reader = BufReader::new(stdout);
    let mut header = String::new();
    reader.read_line(&mut header).await.map_err(|_| {
        InspectionError::new(
            "review_source_unreadable",
            "Git object header could not be read",
        )
    })?;
    let mut fields = header.split_whitespace();
    let _object_id = fields.next();
    let object_type = fields.next();
    let size = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            InspectionError::new("review_source_unavailable", "Git object is unavailable")
        })?;
    if object_type != Some("blob") {
        return Err(InspectionError::new(
            "review_source_unavailable",
            "Git review source is not a blob",
        ));
    }
    Ok((reader, size))
}

async fn skip_git_source(
    reader: &mut BufReader<ChildStdout>,
    offset: u64,
) -> Result<(), InspectionError> {
    let mut skipped = 0u64;
    while skipped < offset {
        // Reuse BufReader's existing buffer: a separate large
        // scratch array across await inflates every enclosing
        // Review future and overflows native worker stacks.
        let available = reader.fill_buf().await.map_err(|_| {
            InspectionError::new("review_source_unreadable", "Git source could not be seeked")
        })?;
        if available.is_empty() {
            return Err(InspectionError::new(
                "review_source_unreadable",
                "Git source could not be seeked",
            ));
        }
        let consumed = (offset - skipped).min(available.len() as u64) as usize;
        reader.consume(consumed);
        skipped += consumed as u64;
    }
    Ok(())
}

async fn read_git_source_page(
    reader: &mut BufReader<ChildStdout>,
    object: String,
    size: u64,
    offset: u32,
) -> Result<FrozenSource, InspectionError> {
    let offset = u64::from(offset).min(size);
    skip_git_source(reader, offset).await?;
    let amount = (size - offset).min(MAX_FILE_BYTES as u64) as usize;
    let mut bytes = vec![0u8; amount];
    reader.read_exact(&mut bytes).await.map_err(|_| {
        InspectionError::new("review_source_unreadable", "Git source could not be read")
    })?;
    let valid_bytes = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error) if error.valid_up_to() > 0 => error.valid_up_to(),
        Err(_) => {
            return Err(InspectionError::new(
                "review_source_boundary",
                "Git source continuation offset is not a UTF-8 boundary",
            ));
        }
    };
    bytes.truncate(valid_bytes);
    let text = String::from_utf8(bytes).map_err(|_| {
        InspectionError::new(
            "review_source_binary",
            "Git review source is not UTF-8 text",
        )
    })?;
    let end = offset.saturating_add(valid_bytes as u64);
    Ok(FrozenSource {
        text: Some(text),
        hash: None,
        identity: Some(object),
        total_lines: None,
        total_bytes: u32::try_from(size).ok(),
        truncated: end < size,
        diagnostic: None,
    })
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
