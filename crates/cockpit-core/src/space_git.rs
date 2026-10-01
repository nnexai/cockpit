//! Branch position of Space checkouts.
//!
//! Herdr reports each Space's checkout but not how far its branch is from the
//! upstream, which its TUI shows as `main ↑14`. Cockpit reads that from Git
//! for the checkout paths in Herdr's snapshot only; clients never name a path.
//! A Space Herdr does not know as a checkout gets the branch of its first
//! pane's folder, as Herdr's sidebar shows it.

use std::collections::HashMap;
use std::path::Path;
use std::process::Output;
use std::time::Duration;

use cockpit_protocol::v1::{
    SessionSnapshotResponse, SpaceGitCheckout, SpaceGitSource, SpaceGitStatus,
    SpaceGitStatusResponse, SpaceGitUpstream, SpaceSummary,
};
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(2);
const GIT_OUTPUT_BYTES: usize = 4096;

/// The folder whose branch a Space shows: its checkout, else its first pane's folder.
pub(crate) fn space_folder<'a>(
    snapshot: &'a SessionSnapshotResponse,
    space: &'a SpaceSummary,
) -> Option<&'a str> {
    if let Some(git) = &space.git {
        return Some(&git.checkout_path);
    }
    snapshot
        .panes
        .iter()
        .filter(|pane| pane.space_id == space.id)
        .find_map(|pane| pane.cwd.as_deref())
}

/// Read once per folder, preserving unavailable metadata rather than inventing empty status.
pub async fn read(snapshot: &SessionSnapshotResponse) -> SpaceGitStatusResponse {
    let mut by_checkout: HashMap<&str, SpaceGitCheckout> = HashMap::new();
    let mut spaces = Vec::new();
    for space in &snapshot.spaces {
        let Some(folder) = space_folder(snapshot, space) else {
            if space.git.is_some() {
                spaces.push(SpaceGitStatus {
                    space_id: space.id.clone(),
                    source: SpaceGitSource::HerdrCheckout,
                    checkout: unavailable(
                        None,
                        "checkout_missing",
                        "Checkout folder is unavailable.",
                    ),
                });
            }
            continue;
        };
        let checkout = match by_checkout.get(folder) {
            Some(status) => status.clone(),
            None => {
                let status = checkout_status(Path::new(folder)).await;
                by_checkout.insert(folder, status.clone());
                status
            }
        };
        if space.git.is_none()
            && matches!(&checkout, SpaceGitCheckout::Unavailable { code, .. }
            if code == "not_a_checkout" || code == "checkout_missing")
        {
            continue;
        }
        spaces.push(SpaceGitStatus {
            space_id: space.id.clone(),
            source: if space.git.is_some() {
                SpaceGitSource::HerdrCheckout
            } else {
                SpaceGitSource::PaneFolder
            },
            checkout,
        });
    }
    SpaceGitStatusResponse {
        session_id: snapshot.session_id.clone(),
        spaces,
    }
}

fn unavailable(root: Option<String>, code: &str, message: &str) -> SpaceGitCheckout {
    SpaceGitCheckout::Unavailable {
        root,
        code: code.to_owned(),
        message: safe_detail(message),
    }
}

/// Bounded, credential-safe diagnostics. Never return a URL or an auth/header value.
pub(crate) fn safe_detail(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Git could not complete the operation.")
        .trim();
    let line = line
        .strip_prefix("fatal: ")
        .or_else(|| line.strip_prefix("error: "))
        .or_else(|| line.strip_prefix("hint: "))
        .unwrap_or(line);
    let lower = line.to_ascii_lowercase();
    if [
        "authorization",
        "password",
        "token",
        "credential",
        "bearer",
        "secret",
        "api key",
        "access key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || line.contains("://")
        || line.contains('@')
        || lower.contains("unable to access")
        || lower.contains("does not appear to be a git repository")
    {
        return "Git could not complete the operation; check authentication and the remote in your terminal.".to_owned();
    }
    line.chars()
        .filter(|ch| !ch.is_control())
        .take(512)
        .collect()
}

pub(crate) struct ResolvedCheckout {
    pub checkout: SpaceGitCheckout,
    pub metadata: Option<BranchMetadata>,
}

pub(crate) async fn checkout_status(checkout: &Path) -> SpaceGitCheckout {
    resolve_checkout(checkout).await.checkout
}

pub(crate) async fn resolve_checkout(checkout: &Path) -> ResolvedCheckout {
    let mut metadata = None;
    let state = async {
        if !checkout.is_absolute() || !checkout.is_dir() {
            return unavailable(None, "checkout_missing", "Checkout folder is unavailable.");
        }
        let output = match git_output(checkout, &["rev-parse", "--show-toplevel"]).await {
            Ok(output) => output,
            Err(error) => return unavailable(None, &error.code, &error.message),
        };
        if !output.status.success() {
            let text = String::from_utf8_lossy(&output.stderr);
            let code = if output.status.code() == Some(128) && text.contains("not a git repository")
            {
                "not_a_checkout"
            } else {
                "git_read_failed"
            };
            return unavailable(None, code, &text);
        }
        let root = match output_line(&output) {
            Some(root) if Path::new(&root).is_absolute() => root,
            _ => {
                return unavailable(
                    None,
                    "git_read_failed",
                    "Git returned an invalid checkout root.",
                );
            }
        };
        let output = match git_output(checkout, &["symbolic-ref", "--quiet", "HEAD"]).await {
            Ok(output) => output,
            Err(error) => return unavailable(Some(root), &error.code, &error.message),
        };
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return SpaceGitCheckout::Detached { root };
        }
        let branch = match output_line(&output)
            .and_then(|line| line.strip_prefix("refs/heads/").map(str::to_owned))
        {
            Some(branch) => branch,
            None => {
                return unavailable(
                    Some(root),
                    "git_read_failed",
                    "Git could not report the local branch.",
                );
            }
        };
        metadata = match branch_metadata(checkout, &branch).await {
            Ok(metadata) => metadata,
            Err(error) => return unavailable(Some(root), &error.code, &error.message),
        };
        let upstream = match metadata.as_ref() {
            None => SpaceGitUpstream::None,
            Some(metadata) if metadata.full_ref.is_empty() => SpaceGitUpstream::None,
            Some(metadata) if metadata.remote == "." => SpaceGitUpstream::Local {
                name: metadata.name.clone(),
            },
            Some(metadata) if metadata.track == "gone" => SpaceGitUpstream::Gone {
                name: metadata.name.clone(),
            },
            Some(metadata) => {
                let revision = format!("{}...HEAD", metadata.full_ref);
                match git_line(
                    checkout,
                    &["rev-list", "--left-right", "--count", &revision],
                )
                .await
                {
                    Ok(line) => match parse_left_right(&line) {
                        Some((behind, ahead)) => SpaceGitUpstream::Tracked {
                            name: metadata.name.clone(),
                            ahead,
                            behind,
                        },
                        None => SpaceGitUpstream::Unavailable {
                            name: metadata.name.clone(),
                            code: "git_read_failed".into(),
                            message: "Git returned invalid upstream counts.".into(),
                        },
                    },
                    Err(error) => SpaceGitUpstream::Unavailable {
                        name: metadata.name.clone(),
                        code: error.code,
                        message: safe_detail(&error.message),
                    },
                }
            }
        };
        SpaceGitCheckout::Branch {
            root,
            branch,
            upstream,
        }
    }
    .await;
    ResolvedCheckout {
        checkout: state,
        metadata,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BranchMetadata {
    pub full_ref: String,
    pub name: String,
    pub remote: String,
    pub remote_ref: String,
    pub track: String,
}

pub(crate) async fn branch_metadata(
    root: &Path,
    branch: &str,
) -> Result<Option<BranchMetadata>, crate::InspectionError> {
    let reference = format!("refs/heads/{branch}");
    let output = git_output(root, &["for-each-ref", "--format=%(refname)%00%(upstream)%00%(upstream:short)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:track,nobracket)", &reference]).await?;
    if !output.status.success() {
        return Err(crate::InspectionError::new(
            "git_read_failed",
            safe_detail(&String::from_utf8_lossy(&output.stderr)),
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| {
        crate::InspectionError::new("git_read_failed", "Git returned invalid branch metadata.")
    })?;
    for line in text.lines() {
        let mut fields = line.split('\0');
        if fields.next() != Some(reference.as_str()) {
            continue;
        }
        let values: Option<[&str; 5]> = (|| {
            Some([
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            ])
        })();
        if let Some([full_ref, name, remote, remote_ref, track]) = values {
            if fields.next().is_none() {
                return Ok(Some(BranchMetadata {
                    full_ref: full_ref.into(),
                    name: name.into(),
                    remote: remote.into(),
                    remote_ref: remote_ref.into(),
                    track: track.into(),
                }));
            }
        }
        return Err(crate::InspectionError::new(
            "git_read_failed",
            "Git returned invalid branch metadata.",
        ));
    }
    Ok(None)
}

/// Parse `git rev-list --left-right --count upstream...HEAD` output: "behind\tahead".
fn parse_left_right(line: &str) -> Option<(u32, u32)> {
    let mut parts = line.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let ahead = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((behind, ahead))
}

fn output_line(output: &Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }
    let text = std::str::from_utf8(&output.stdout).ok()?.trim();
    (!text.is_empty() && !text.contains('\n')).then(|| text.to_owned())
}

pub(crate) async fn git_line(
    directory: &Path,
    args: &[&str],
) -> Result<String, crate::InspectionError> {
    let output = git_output(directory, args).await?;
    output_line(&output).ok_or_else(|| {
        crate::InspectionError::new(
            "git_read_failed",
            safe_detail(&String::from_utf8_lossy(&output.stderr)),
        )
    })
}

pub(crate) async fn git_output(
    directory: &Path,
    args: &[&str],
) -> Result<Output, crate::InspectionError> {
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
        .env("GIT_ASKPASS", "")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY");
    crate::process::run_bounded_command(
        command,
        GIT_OUTPUT_BYTES,
        GIT_OUTPUT_BYTES,
        GIT_TIMEOUT,
        "git",
    )
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use cockpit_protocol::v1::{PaneSummary, SpaceGitSummary};
    use uuid::Uuid;

    use super::*;

    pub(crate) fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git starts");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8")
    }

    pub(crate) fn commit(root: &Path, message: &str) {
        git(root, &["commit", "--allow-empty", "-m", message]);
    }

    pub(crate) fn space(id: &str, checkout: &Path) -> SpaceSummary {
        SpaceSummary {
            id: id.to_owned(),
            label: id.to_owned(),
            number: 1,
            tab_count: 1,
            pane_count: 1,
            focused: false,
            agent_status: "idle".to_owned(),
            git: Some(SpaceGitSummary {
                repository_key: "key".to_owned(),
                repository: "sample".to_owned(),
                branch: None,
                checkout_path: checkout.to_string_lossy().into_owned(),
                is_linked_worktree: false,
            }),
        }
    }

    pub(crate) fn snapshot(spaces: Vec<SpaceSummary>) -> SessionSnapshotResponse {
        SessionSnapshotResponse {
            session_id: "session".to_owned(),
            server_instance: "0123456789abcdef".into(),
            version: "0.9.1".to_owned(),
            protocol: 22,
            focused_space_id: None,
            focused_tab_id: None,
            herdr_shell: None,
            focused_pane_id: None,
            spaces,
            tabs: Vec::new(),
            panes: Vec::new(),
            agents: Vec::new(),
        }
    }

    /// A clone of a bare origin with `main` tracking `origin/main`.
    pub(crate) fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("cockpit-space-git-{}", Uuid::new_v4()));
        let seed = root.join("seed");
        let origin = root.join("origin.git");
        let clone = root.join("clone");
        std::fs::create_dir_all(&seed).expect("seed directory");
        git(&seed, &["init", "--initial-branch=main"]);
        git(&seed, &["config", "user.email", "fixture@example.test"]);
        git(&seed, &["config", "user.name", "Fixture"]);
        commit(&seed, "base");
        git(&root, &["clone", "--bare", "seed", "origin.git"]);
        git(&root, &["clone", "origin.git", "clone"]);
        git(&clone, &["config", "user.email", "fixture@example.test"]);
        git(&clone, &["config", "user.name", "Fixture"]);
        (root, origin, clone)
    }

    #[tokio::test]
    async fn counts_commits_ahead_of_and_behind_the_upstream() {
        let (root, _origin, clone) = fixture();
        commit(&clone, "local one");
        commit(&clone, "local two");
        let other = root.join("other");
        git(&root, &["clone", "origin.git", "other"]);
        git(&other, &["config", "user.email", "fixture@example.test"]);
        git(&other, &["config", "user.name", "Fixture"]);
        commit(&other, "remote");
        git(&other, &["push", "origin", "main"]);
        git(&clone, &["fetch", "origin"]);

        let status = read(&snapshot(vec![space("w1", &clone)])).await;

        assert_eq!(status.session_id, "session");
        assert_eq!(
            status.spaces,
            vec![SpaceGitStatus {
                space_id: "w1".to_owned(),
                source: SpaceGitSource::HerdrCheckout,
                checkout: SpaceGitCheckout::Branch {
                    root: clone.to_string_lossy().into_owned(),
                    branch: "main".into(),
                    upstream: SpaceGitUpstream::Tracked {
                        name: "origin/main".into(),
                        ahead: 2,
                        behind: 1
                    },
                },
            }]
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn distinguishes_no_upstream_and_missing_checkout() {
        let (root, _origin, clone) = fixture();
        git(&clone, &["switch", "--quiet", "-c", "topic"]);
        let missing = root.join("missing");

        let status = read(&snapshot(vec![space("w1", &clone), space("w2", &missing)])).await;

        assert!(
            matches!(&status.spaces[0].checkout, SpaceGitCheckout::Branch { branch, upstream: SpaceGitUpstream::None, .. } if branch == "topic")
        );
        assert!(
            matches!(&status.spaces[1].checkout, SpaceGitCheckout::Unavailable { root: None, code, .. } if code == "checkout_missing")
        );
        std::fs::remove_dir_all(root).ok();
    }

    pub(crate) fn pane(space_id: &str, cwd: &Path) -> PaneSummary {
        PaneSummary {
            id: format!("{space_id}:p1"),
            terminal_id: format!("term-{space_id}"),
            space_id: space_id.to_owned(),
            tab_id: format!("{space_id}:t1"),
            title: None,
            focused: false,
            agent: None,
            agent_status: "unknown".to_owned(),
            revision: 0,
            cwd: Some(cwd.to_string_lossy().into_owned()),
        }
    }

    #[tokio::test]
    async fn a_plain_space_shows_the_branch_of_its_first_panes_folder() {
        let (root, _origin, clone) = fixture();
        let nested = clone.join("nested");
        std::fs::create_dir_all(&nested).expect("nested folder");
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).expect("outside folder");
        let plain = |id: &str| SpaceSummary {
            git: None,
            ..space(id, &clone)
        };
        let mut state = snapshot(vec![plain("w1"), plain("w2"), plain("w3")]);
        state.panes = vec![pane("w1", &nested), pane("w2", &outside)];

        let status = read(&state).await;

        assert_eq!(
            status.spaces.len(),
            1,
            "no branch outside Git or without a pane"
        );
        assert_eq!(status.spaces[0].space_id, "w1");
        assert_eq!(status.spaces[0].source, SpaceGitSource::PaneFolder);
        assert!(
            matches!(&status.spaces[0].checkout, SpaceGitCheckout::Branch { root, branch, upstream: SpaceGitUpstream::Tracked { name, .. } }
            if root.as_str() == clone.to_str().unwrap() && branch == "main" && name == "origin/main")
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn distinguishes_detached_local_gone_and_git_failure() {
        let (root, origin, clone) = fixture();
        git(&clone, &["switch", "--detach"]);
        assert!(matches!(
            checkout_status(&clone).await,
            SpaceGitCheckout::Detached { .. }
        ));
        git(&clone, &["switch", "main"]);
        git(&clone, &["branch", "local"]);
        git(&clone, &["branch", "--set-upstream-to=local", "main"]);
        assert!(matches!(
            checkout_status(&clone).await,
            SpaceGitCheckout::Branch {
                upstream: SpaceGitUpstream::Local { .. },
                ..
            }
        ));
        git(&clone, &["branch", "--set-upstream-to=origin/main", "main"]);
        git(&origin, &["update-ref", "-d", "refs/heads/main"]);
        git(&clone, &["fetch", "--prune"]);
        assert!(
            matches!(checkout_status(&clone).await, SpaceGitCheckout::Branch { upstream: SpaceGitUpstream::Gone { name }, .. } if name == "origin/main")
        );
        std::fs::write(clone.join(".git/config"), "[broken").expect("corrupt local config");
        assert!(
            matches!(checkout_status(&clone).await, SpaceGitCheckout::Unavailable { code, .. } if code != "not_a_checkout")
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn parses_left_right_counts() {
        assert_eq!(parse_left_right("3\t14"), Some((3, 14)));
        assert_eq!(parse_left_right("3"), None);
        assert_eq!(parse_left_right("a\t1"), None);
    }
}
