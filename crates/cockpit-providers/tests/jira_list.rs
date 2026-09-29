#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cockpit_core::sources::{IssueQuery, SourceProvider};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
use cockpit_providers::jira::JiraSourceProvider;
use serde_json::{Value, json};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

/// A `jira` that answers `issue list` from `state.json`: it understands the
/// guard prefix, `key in (…)`, `updated >= "…"` and `updated < "…"`, orders
/// by `updated` descending (stable), and logs every argv.
const FAKE: &str = r#"#!/usr/bin/env python3
import json, pathlib, re, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'argv.jsonl').open('a') as log:
    log.write(json.dumps(args) + '\n')
state = json.loads((root / 'state.json').read_text())
assert args[:2] == ['issue', 'list'], args
jql = args[args.index('--jql') + 1]
assert jql.startswith('project IS NOT EMPTY AND ('), jql
assert args[args.index('--delimiter') + 1] == '\x1f'
limit = int(args[args.index('--paginate') + 1].split(':')[1])
issues = state['issues']
keys = re.search(r'key in \(([^)]*)\)', jql)
if keys:
    wanted = [k.strip() for k in keys.group(1).split(',')]
    if any(k in state.get('rejected', []) for k in wanted):
        sys.stderr.write("jira: Received unexpected response '400 Bad Request'.\n")
        sys.exit(1)
    issues = [i for i in issues if i['key'] in wanted]
lower = re.search(r'updated >= "([^"]+)"', jql)
upper = re.search(r'updated < "([^"]+)"', jql)
if lower:
    issues = [i for i in issues if i['updated'][:16] >= lower.group(1)]
if upper:
    issues = [i for i in issues if i['updated'][:16] < upper.group(1)]
issues = sorted(issues, key=lambda i: i['updated'], reverse=True)[:limit]
if not issues:
    sys.stderr.write('\x1b[0;31m\u2717\x1b[0m No result found for given query in project "OPS"\n')
    sys.exit(1)
for i in issues:
    print('\x1f'.join([i['key'], i['updated'], 'To Do', 'Task', i.get('assignee', '')]))
"#;

struct Fixture {
    root: PathBuf,
    provider: JiraSourceProvider,
}

impl Fixture {
    fn new(issues: Vec<Value>, rejected: &[&str]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cockpit-jira-list-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let command = root.join("jira");
        std::fs::write(&command, FAKE).unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            root.join("state.json"),
            serde_json::to_vec(&json!({"issues": issues, "rejected": rejected})).unwrap(),
        )
        .unwrap();
        let configuration = ProjectConfiguration {
            version: 1,
            repository_roots: vec![],
            worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
            companion_root: root.join("companions").to_string_lossy().into_owned(),
            state_root: root.join("state").to_string_lossy().into_owned(),
            cache_root: root.join("cache").to_string_lossy().into_owned(),
            library_root: root.join("library").to_string_lossy().into_owned(),
            branch_template: "{repo}/{task_id}".into(),
            checkout_template: "{repo}-{task_id}".into(),
            providers: vec![ProjectProvider {
                id: "jira".into(),
                base_url: "https://jira.test".into(),
                executable: command.to_string_lossy().into_owned(),
                login: None,
            }],
            limits: ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 5000,
                git_output_bytes: 1024 * 1024,
                operation_timeout_ms: 20_000,
                context_preview_bytes: 1024,
                context_preview_lines: 100,
                context_directory_entries: 100,
                context_tree_depth: 4,
                library_folder_files: 512,
                library_folder_bytes: 32 * 1024 * 1024,
                library_file_bytes: 4 * 1024 * 1024,
                library_space_pages: 200,
                library_attachment_bytes: 25 * 1024 * 1024,
                library_item_attachment_bytes: 100 * 1024 * 1024,
                library_max_items: 20_000,
            },
            origins: Default::default(),
        };
        let provider = JiraSourceProvider::configured(&configuration, "jira").unwrap();
        Self { root, provider }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.root.join("argv.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn jql(&self, call: usize) -> String {
        let argv = &self.calls()[call];
        argv[argv.iter().position(|arg| arg == "--jql").unwrap() + 1].clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Issue `n` was updated `n` minutes after 2026-03-01 00:00, or all in the
/// same minute when `same_minute`.
fn issues(count: usize, same_minute: bool) -> Vec<Value> {
    (0..count)
        .map(|n| {
            let minute = if same_minute { 0 } else { n };
            json!({"key": format!("OPS-{}", n + 1),
                   "updated": format!("2026-03-01 {:02}:{:02}:{:02}", minute / 60, minute % 60, n % 60)})
        })
        .collect()
}

fn jql_query(jql: &str) -> IssueQuery<'_> {
    IssueQuery::Jql {
        jql,
        updated_since: None,
    }
}

#[tokio::test]
async fn jira_windows_past_one_hundred_rows_and_reports_complete() {
    let fixture = Fixture::new(issues(250, false), &[]);
    let cancel = AtomicBool::new(false);
    let listing = fixture
        .provider
        .list_issues(&jql_query("project = OPS"), 1000, &cancel)
        .await
        .unwrap();
    assert_eq!(listing.rows.len(), 250);
    assert!(listing.complete);
    assert_eq!(listing.rows[0].key, "OPS-250");
    assert_eq!(fixture.calls().len(), 3);
    assert_eq!(fixture.jql(0), "project IS NOT EMPTY AND (project = OPS)");
    assert!(fixture.jql(1).contains("AND updated < \"2026-03-01 02:31\""));

    let capped = fixture
        .provider
        .list_issues(&jql_query("project = OPS"), 120, &cancel)
        .await
        .unwrap();
    assert_eq!(capped.rows.len(), 120);
    assert!(!capped.complete);
}

#[tokio::test]
async fn jira_more_than_a_page_in_one_minute_is_incomplete() {
    let fixture = Fixture::new(issues(150, true), &[]);
    let listing = fixture
        .provider
        .list_issues(&jql_query("project = OPS"), 1000, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(listing.rows.len(), 100);
    assert!(!listing.complete);
}

#[tokio::test]
async fn jira_no_result_found_is_an_empty_complete_listing() {
    let fixture = Fixture::new(vec![], &[]);
    let listing = fixture
        .provider
        .list_issues(&jql_query("project = OPS"), 10, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(listing.rows.is_empty());
    assert!(listing.complete);
}

#[tokio::test]
async fn jira_probe_starts_one_minute_before_the_watermark() {
    let fixture = Fixture::new(issues(250, false), &[]);
    let listing = fixture
        .provider
        .list_issues(
            &IssueQuery::Jql {
                jql: "project = OPS",
                updated_since: Some("2026-03-01T02:30:10.000+0100"),
            },
            1000,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
    // Minutes 02:29 (issue 150) through 04:09 (issue 250).
    assert_eq!(listing.rows.len(), 101);
    assert!(listing.complete);
    assert_eq!(
        fixture.jql(0),
        "project IS NOT EMPTY AND ((project = OPS) AND updated >= \"2026-03-01 02:29\")"
    );
}

#[tokio::test]
async fn jira_a_rejected_key_batch_bisects_to_the_missing_key() {
    let fixture = Fixture::new(issues(4, false), &["OPS-3"]);
    let keys: Vec<String> = (1..=5).map(|n| format!("OPS-{n}")).collect();
    let listing = fixture
        .provider
        .list_issues(&IssueQuery::Keys(&keys), 5, &AtomicBool::new(false))
        .await
        .unwrap();
    let mut found: Vec<&str> = listing.rows.iter().map(|row| row.key.as_str()).collect();
    found.sort();
    assert_eq!(found, ["OPS-1", "OPS-2", "OPS-4"]);
    assert!(listing.complete);
}
