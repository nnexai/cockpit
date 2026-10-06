#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use cockpit_core::sources::{SourceFetchRequest, SourceProvider, instance_authority};
use cockpit_protocol::projects::{
    ProjectConfiguration, ProjectLimits, ProjectProvider, ProviderKind,
};
use cockpit_providers::{github::GithubSourceProvider, gitlab::GitlabSourceProvider};
use serde_json::{Value, json};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    config: ProjectConfiguration,
    data: Value,
}

impl Fixture {
    fn new(kind: ProviderKind, executable: &str, base: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cockpit-provider-contract-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let command = root.join(executable);
        std::fs::write(
            &command,
            r#"#!/usr/bin/env python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
data = json.loads((root / 'responses.json').read_text())
with (root / 'argv.jsonl').open('a') as log:
    log.write(json.dumps(args) + '\n')
name = pathlib.Path(__file__).name
if name == 'gh':
    if args[0] in ('pr', 'issue'):
        assert args[1:5] == ['view', '7', '--repo', 'other/repo']
        print(json.dumps(data['artifact']))
    else:
        assert args[0] == 'api' and args[args.index('--method') + 1] == 'GET'
        assert args[args.index('--hostname') + 1] == 'github.com'
        kind = 'review_pages' if '/pulls/' in args[1] else 'conversation_pages'
        page = int(next(arg[5:] for arg in args if arg.startswith('page=')))
        print('HTTP/1.1 200 OK')
        if page < len(data[kind]):
            print('Link: <https://api.github.com/' + args[1] + '?page=2>; rel="next"')
        print()
        print(json.dumps(data[kind][page - 1]))
elif name == 'glab':
    assert args[0] == 'api' and 'GET' in args
    assert args[args.index('--hostname') + 1] == 'gitlab.test'
    endpoint = next(arg for arg in args if arg.startswith('https://'))
    assert endpoint.startswith('https://gitlab.test:9443/subfolder/api/v4/projects/')
    if '/notes?' in endpoint or '/discussions?' in endpoint:
        print('[]')
    elif endpoint.endswith('/approvals'):
        print(json.dumps(data['gitlab_approvals']))
    elif '/merge_requests/' in endpoint:
        print(json.dumps(data['gitlab_review']))
    elif '/issues/' in endpoint:
        print(json.dumps(data['gitlab_issue']))
    else:
        print(json.dumps(data['gitlab_project']))
"#,
        )
        .unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = ProjectConfiguration { version: 1, orchestration: Default::default(), repository_roots: vec![],
        worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
        companion_root: root.join("companions").to_string_lossy().into_owned(),
        state_root: root.join("state").to_string_lossy().into_owned(),
        cache_root: root.join("cache").to_string_lossy().into_owned(),
        library_root: root.join("library").to_string_lossy().into_owned(),
        notes_root: root.join("notes").to_string_lossy().into_owned(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: vec![ProjectProvider {
            id: "fixture".into(),
            kind,
            base_url: base.into(),
            executable: Some(command.to_string_lossy().into_owned()),
            login: None,
            deployment: None,
        }],
        limits: ProjectLimits {
            catalog_depth: 1,
            catalog_entries: 1,
            git_timeout_ms: 5000,
            git_output_bytes: 1024 * 1024,
            operation_timeout_ms: 5000,
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
        origins: Default::default(), };
        let data = serde_json::from_str(include_str!("fixtures/source_contracts.json")).unwrap();
        let fixture = Self { root, config, data };
        fixture.save();
        fixture
    }

    fn save(&self) {
        std::fs::write(
            self.root.join("responses.json"),
            serde_json::to_vec(&self.data).unwrap(),
        )
        .unwrap();
    }

    fn request(&self, url: &str) -> SourceFetchRequest {
        SourceFetchRequest {
            provider_id: "fixture".into(),
            artifact_url: url.into(),
            authority: instance_authority(&self.config, "fixture", url).unwrap(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn github_prs_preserve_head_and_paginate_both_comment_kinds_without_inventing_fork_branches()
{
    let mut fixture = Fixture::new(ProviderKind::Github, "gh", "https://github.com");
    let provider = GithubSourceProvider::configured(&fixture.config, "fixture").unwrap();
    let request = fixture.request("https://github.com/other/repo/pull/7");
    assert_eq!(request.authority.owner, "other");
    assert_eq!(request.authority.repository, "repo");
    for (record, branch) in [
        ("github_same_repo", Some("feature/review")),
        ("github_fork", None),
    ] {
        fixture.data["artifact"] = fixture.data[record].clone();
        fixture.save();
        let metadata = provider.metadata(&request).await.unwrap();
        assert_eq!(metadata.source_branch.as_deref(), branch);
        assert_eq!(
            metadata.source_url.as_deref(),
            Some(request.artifact_url.as_str())
        );
        let assets = provider.fetch(&request).await.unwrap();
        assert_eq!(assets.len(), 1);
        let asset = &assets[0];
        assert_eq!(asset.source.resource_type, "review");
        assert_eq!(asset.source.canonical_id, "other/repo!7");
        assert_eq!(asset.source.provider_instance, "https://github.com");
        assert_eq!(asset.source_revision, metadata.source_commit);
        assert_eq!(asset.container.as_ref().unwrap().id, "other/repo");
        for text in [
            "Conversation first page",
            "Conversation second page",
            "Review first page",
            "Review second page",
            "## Comments (",
            " · review on src/main.rs:12\n[#201](",
        ] {
            assert!(asset.body.contains(text), "missing {text}");
        }
    }
    for url in [
        "https://github.com/wrong/repo/pull/7",
        "https://github.com/other/repo/pull/8",
        "http://github.com/other/repo/pull/7",
        "https://github.com:9443/other/repo/pull/7",
    ] {
        fixture.data["artifact"]["url"] = json!(url);
        fixture.save();
        assert_eq!(
            provider.fetch(&request).await.unwrap_err().code,
            "source_identity_mismatch"
        );
        assert_eq!(
            provider.metadata(&request).await.unwrap_err().code,
            "source_identity_mismatch"
        );
    }
}

#[tokio::test]
async fn github_issue_canonical_url_is_verified_too() {
    let mut fixture = Fixture::new(ProviderKind::Github, "gh", "https://github.com");
    fixture.data["artifact"] = json!({"number":7,"title":"Issue","body":"Issue body","url":"https://github.com/other/repo/issues/7","updatedAt":"2026-09-26T12:00:00Z"});
    fixture.save();
    let provider = GithubSourceProvider::configured(&fixture.config, "fixture").unwrap();
    let request = fixture.request("https://github.com/other/repo/issues/7");
    let assets = provider.fetch(&request).await.unwrap();
    assert_eq!(assets[0].source.canonical_id, "other/repo#7");
    assert_eq!(
        assets[0].source_url.as_deref(),
        Some(request.artifact_url.as_str())
    );
    fixture.data["artifact"]["url"] = json!("https://github.com/wrong/repo/issues/7");
    fixture.save();
    assert_eq!(
        provider.fetch(&request).await.unwrap_err().code,
        "source_identity_mismatch"
    );
}

#[tokio::test]
async fn self_hosted_gitlab_issue_and_review_keep_port_base_path_and_cross_repository_identity() {
    let mut fixture = Fixture::new(ProviderKind::Gitlab, "glab", "https://gitlab.test:9443/subfolder");
    let provider = GitlabSourceProvider::configured(&fixture.config, "fixture").unwrap();
    for (path, kind, separator, record) in [
        ("issues", "issue", '#', "gitlab_issue"),
        ("merge_requests", "review", '!', "gitlab_review"),
    ] {
        let url = format!("https://gitlab.test:9443/subfolder/other/group/repo/-/{path}/7");
        let request = fixture.request(&url);
        assert_eq!(request.authority.owner, "other/group");
        assert_eq!(request.authority.repository, "repo");
        assert_eq!(request.authority.origin_port, Some(9443));
        assert_eq!(request.authority.origin_base_path, "/subfolder");
        let assets = provider.fetch(&request).await.unwrap();
        assert_eq!(assets[0].source.resource_type, kind);
        assert_eq!(
            assets[0].source.canonical_id,
            format!("other/group/repo{separator}7")
        );
        assert_eq!(
            assets[0].source.provider_instance,
            "https://gitlab.test:9443/subfolder"
        );
        assert_eq!(assets[0].source_url.as_deref(), Some(url.as_str()));
        assert_eq!(assets[0].container.as_ref().unwrap().id, "other/group/repo");
        let original = fixture.data[record]["web_url"].clone();
        for mismatch in [
            url.replace(":9443", ""),
            url.replace("/subfolder/", "/elsewhere/"),
            url.replace("other/group/repo", "another/repo"),
        ] {
            fixture.data[record]["web_url"] = json!(mismatch);
            fixture.save();
            assert_eq!(
                provider.fetch(&request).await.unwrap_err().code,
                "source_identity_mismatch"
            );
        }
        fixture.data[record]["web_url"] = original;
        fixture.save();
    }
}


#[test]
fn instance_authority_uses_artifact_repository_for_nested_wiki_pages() {
    let fixture = Fixture::new(ProviderKind::Gitea, "tea", "https://forge.test:9443/gitea");
    let url = "https://forge.test:9443/gitea/other/repo/wiki/design:proposal/details";
    let authority = instance_authority(&fixture.config, "fixture", url).unwrap();
    assert_eq!(authority.owner, "other");
    assert_eq!(authority.repository, "repo");
    assert_eq!(authority.provider_instance, "https://forge.test:9443/gitea");
    assert_eq!(authority.origin_port, Some(9443));
    assert_eq!(
        instance_authority(&fixture.config, "another-provider", url)
            .unwrap_err()
            .code,
        "source_authority_mismatch"
    );
}
