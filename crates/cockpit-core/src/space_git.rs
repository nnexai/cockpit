//! Branch position of Space checkouts.
//!
//! Herdr reports each Space's checkout but not how far its branch is from the
//! upstream, which its TUI shows as `main ↑14`. Cockpit reads that from Git
//! for the checkout paths in Herdr's snapshot only; clients never name a path.

use std::collections::HashMap;
use std::path::Path;
use std::process::Output;
use std::time::Duration;

use cockpit_protocol::v1::{SessionSnapshotResponse, SpaceGitStatus, SpaceGitStatusResponse};
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(2);
const GIT_OUTPUT_BYTES: usize = 4096;

#[derive(Clone)]
struct CheckoutStatus {
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u32>,
    behind: Option<u32>,
}

/// Read branch, upstream and ahead/behind counts for every Space with a checkout.
/// Git failures leave the fields empty: the status is informational.
pub async fn read(snapshot: &SessionSnapshotResponse) -> SpaceGitStatusResponse {
    let mut by_checkout: HashMap<&str, CheckoutStatus> = HashMap::new();
    let mut spaces = Vec::new();
    for space in &snapshot.spaces {
        let Some(git) = &space.git else { continue };
        let status = match by_checkout.get(git.checkout_path.as_str()) {
            Some(status) => status.clone(),
            None => {
                let status = checkout_status(Path::new(&git.checkout_path)).await;
                by_checkout.insert(&git.checkout_path, status.clone());
                status
            }
        };
        spaces.push(SpaceGitStatus {
            space_id: space.id.clone(),
            branch: status.branch,
            upstream: status.upstream,
            ahead: status.ahead,
            behind: status.behind,
        });
    }
    SpaceGitStatusResponse {
        session_id: snapshot.session_id.clone(),
        spaces,
    }
}

async fn checkout_status(checkout: &Path) -> CheckoutStatus {
    let empty = CheckoutStatus {
        branch: None,
        upstream: None,
        ahead: None,
        behind: None,
    };
    if !checkout.is_absolute() || !checkout.is_dir() {
        return empty;
    }
    let branch = git_line(checkout, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await;
    let upstream = git_line(
        checkout,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .await;
    let counts = match upstream {
        Some(_) => git_line(
            checkout,
            &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
        )
        .await
        .and_then(|line| parse_left_right(&line)),
        None => None,
    };
    CheckoutStatus {
        branch,
        upstream,
        behind: counts.map(|(behind, _)| behind),
        ahead: counts.map(|(_, ahead)| ahead),
    }
}

/// Parse `git rev-list --left-right --count upstream...HEAD` output: "behind\tahead".
fn parse_left_right(line: &str) -> Option<(u32, u32)> {
    let mut parts = line.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let ahead = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((behind, ahead))
}

async fn git_line(directory: &Path, args: &[&str]) -> Option<String> {
    let output = git_output(directory, args).await.ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let line = text.trim();
    (!line.is_empty() && !line.contains('\n')).then(|| line.to_owned())
}

async fn git_output(directory: &Path, args: &[&str]) -> Result<Output, crate::InspectionError> {
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
mod tests {
    use std::path::PathBuf;

    use cockpit_protocol::v1::{SpaceGitSummary, SpaceSummary};
    use uuid::Uuid;

    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git starts");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8")
    }

    fn commit(root: &Path, message: &str) {
        git(root, &["commit", "--allow-empty", "-m", message]);
    }

    fn space(id: &str, checkout: &Path) -> SpaceSummary {
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

    fn snapshot(spaces: Vec<SpaceSummary>) -> SessionSnapshotResponse {
        SessionSnapshotResponse {
            session_id: "session".to_owned(),
            version: "0.9.1".to_owned(),
            protocol: 22,
            focused_space_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            spaces,
            tabs: Vec::new(),
            panes: Vec::new(),
            agents: Vec::new(),
            layouts: Vec::new(),
        }
    }

    /// A clone of a bare origin with `main` tracking `origin/main`.
    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
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
                branch: Some("main".to_owned()),
                upstream: Some("origin/main".to_owned()),
                ahead: Some(2),
                behind: Some(1),
            }]
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn leaves_counts_empty_without_an_upstream_or_checkout() {
        let (root, _origin, clone) = fixture();
        git(&clone, &["switch", "--quiet", "-c", "topic"]);
        let missing = root.join("missing");

        let status = read(&snapshot(vec![space("w1", &clone), space("w2", &missing)])).await;

        assert_eq!(status.spaces[0].branch.as_deref(), Some("topic"));
        assert_eq!(status.spaces[0].upstream, None);
        assert_eq!(status.spaces[0].ahead, None);
        assert_eq!(status.spaces[1].branch, None);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn parses_left_right_counts() {
        assert_eq!(parse_left_right("3\t14"), Some((3, 14)));
        assert_eq!(parse_left_right("3"), None);
        assert_eq!(parse_left_right("a\t1"), None);
    }
}
