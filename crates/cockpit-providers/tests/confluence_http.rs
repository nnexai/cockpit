//! Consumer-boundary coverage against real HTTP, without any installed CLI.
#[path = "support/fake_confluence.rs"]
mod fake_confluence;

use cap_std::{ambient_authority, fs::Dir};
use cockpit_core::credentials::{MemoryVault, ProviderCredentials};
use cockpit_core::process::StagingBudget;
use cockpit_core::sources::{
    AttachmentRef, FrontmatterValue, ProviderResolution, SourceAsset, SourceAuthority,
    SourceFetchRequest, SourceProvider, SourceService, confluence_page_url,
};
use cockpit_protocol::credentials::{ProviderAuthKind, ProviderCredentialSetRequest};
use cockpit_protocol::projects::{
    ProjectConfiguration, ProjectLimits, ProjectProvider, ProviderDeployment, ProviderKind,
};
use cockpit_providers::{confluence::ConfluenceSourceProvider, credential_kinds};
use fake_confluence::{FakeConfluence, Mode, Page, Response, TOKEN, page_json};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use url::Url;

fn configuration(server: &FakeConfluence) -> ProjectConfiguration {
    ProjectConfiguration {
        repository_roots: Vec::new(),
        worktree_root: "/w".into(),
        state_root: "/s".into(),
        library_root: "/l".into(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: vec![ProjectProvider {
            id: "wiki".into(),
            kind: ProviderKind::Confluence,
            base_url: server.base_url(),
            executable: None,
            login: None,
            deployment: Some(match server.mode {
                Mode::Cloud => ProviderDeployment::Cloud,
                Mode::DataCenter => ProviderDeployment::DataCenter,
            }),
        }],
        limits: ProjectLimits {
            catalog_depth: 1,
            catalog_entries: 1,
            git_timeout_ms: 5000,
            git_output_bytes: 1024 * 1024,
            operation_timeout_ms: 10_000,
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
        ..ProjectConfiguration::for_tests(std::path::Path::new("/"))
    }
}
async fn provider(
    server: &FakeConfluence,
    token: bool,
) -> (ConfluenceSourceProvider, Arc<ProviderCredentials>) {
    let config = configuration(server);
    let credentials = Arc::new(ProviderCredentials::new(
        &config,
        Arc::new(MemoryVault::new()),
        credential_kinds,
    ));
    if token {
        credentials
            .set(ProviderCredentialSetRequest {
                provider_id: "wiki".into(),
                kind: ProviderAuthKind::Bearer,
                username: None,
                token: TOKEN.into(),
            })
            .await
            .unwrap();
    }
    (
        ConfluenceSourceProvider::configured(&config, "wiki", credentials.clone()).unwrap(),
        credentials,
    )
}
fn request(server: &FakeConfluence, id: &str) -> SourceFetchRequest {
    let base = server.base_url();
    let url = Url::parse(&base).unwrap();
    SourceFetchRequest {
        provider_id: "wiki".into(),
        artifact_url: confluence_page_url(&base, id),
        authority: SourceAuthority {
            provider_instance: base,
            origin_host: url.host_str().unwrap().into(),
            origin_port: url.port(),
            origin_base_path: url.path().into(),
            owner: String::new(),
            repository: String::new(),
        },
    }
}
fn fixture(mode: Mode) -> FakeConfluence {
    let server = FakeConfluence::start(mode);
    let mut root = Page::new("10", "Engineering Home", &[]);
    root.attachments.clear();
    server.add_page(root);
    let ancestors = if mode == Mode::Cloud {
        vec![
            ("10", "page", "Engineering Home"),
            ("20", "folder", "Release Folder"),
        ]
    } else {
        vec![("10", "page", "Engineering Home")]
    };
    server.add_page(Page::new("30", "Release Checklist", &ancestors));
    let mut sibling = Page::new("31", "Sibling Page", &ancestors);
    sibling.attachments.clear();
    server.add_page(sibling);
    let mut nested = ancestors;
    nested.push(("30", "page", "Release Checklist"));
    let mut child = Page::new("40", "Child Page", &nested);
    child.attachments.clear();
    server.add_page(child);
    server
}
fn field<'a>(asset: &'a SourceAsset, key: &str) -> Option<&'a FrontmatterValue> {
    asset
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
}
fn text(value: &str) -> FrontmatterValue {
    FrontmatterValue::String(value.into())
}
fn strings(values: &[&str]) -> FrontmatterValue {
    FrontmatterValue::Strings(values.iter().map(|value| value.to_string()).collect())
}
fn path(server: &FakeConfluence, suffix: &str) -> String {
    format!("{}{suffix}", server.mode.api())
}
fn attachment() -> AttachmentRef {
    AttachmentRef {
        id: "att557057".into(),
        title: "release-flow.png".into(),
        bytes: Some(4),
    }
}
static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
struct Destination {
    path: PathBuf,
    dir: Dir,
}
impl Destination {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cockpit-confluence-http-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        Self { path, dir }
    }
}
impl Drop for Destination {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn budget(bytes: u64) -> StagingBudget {
    StagingBudget {
        bytes,
        max_files: 1,
    }
}

#[tokio::test]
async fn cloud_and_dc_fetch_preserve_library_fields_hierarchy_and_attachment_metadata() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let assets = provider.fetch(&request(&server, "30")).await.unwrap();
        let asset = &assets[0];
        assert_eq!(asset.source.canonical_id, "30");
        assert_eq!(asset.source.resource_type, "page");
        assert_eq!(asset.source.provider_instance, server.base_url());
        assert_eq!(asset.title, "Release Checklist");
        assert_eq!(asset.source_revision.as_deref(), Some("7"));
        assert!(asset.complete);
        assert!(asset.diagnostics.is_empty());
        assert_eq!(asset.body, "# Overview\n\nHello **world**.");
        assert_eq!(field(asset, "space_key"), Some(&text("ENG")));
        assert_eq!(field(asset, "space_name"), Some(&text("Engineering")));
        assert_eq!(field(asset, "page_id"), Some(&text("30")));
        assert_eq!(field(asset, "version"), Some(&FrontmatterValue::Number(7)));
        assert_eq!(
            field(asset, "last_modified"),
            Some(&text("2026-09-25T14:03:11.000Z"))
        );
        assert_eq!(
            field(asset, "last_modified_by"),
            Some(&text("Fixture Author"))
        );
        assert_eq!(
            field(asset, "labels"),
            Some(&strings(&["engineering", "release"]))
        );
        let expected_ids = if mode == Mode::Cloud {
            vec!["10", "20"]
        } else {
            vec!["10"]
        };
        let expected_titles = if mode == Mode::Cloud {
            vec!["Engineering Home", "Release Folder"]
        } else {
            vec!["Engineering Home"]
        };
        assert_eq!(field(asset, "ancestor_ids"), Some(&strings(&expected_ids)));
        assert_eq!(field(asset, "ancestors"), Some(&strings(&expected_titles)));
        assert_eq!(
            field(asset, "parent_id"),
            Some(&text(expected_ids.last().unwrap()))
        );
        assert_eq!(asset.container.as_ref().unwrap().label, "ENG · Engineering");
        let listed = provider
            .list_space_pages("ENG", 100, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(listed.complete);
        assert_eq!(listed.pages.len(), 4);
        assert_eq!(listed.homepage_id.as_deref(), Some("10"));
        assert_eq!(
            listed
                .pages
                .iter()
                .find(|page| page.page_id == "30")
                .unwrap()
                .ancestors,
            expected_ids
        );
        assert_eq!(
            listed
                .pages
                .iter()
                .find(|page| page.page_id == "40")
                .unwrap()
                .ancestors,
            if mode == Mode::Cloud {
                vec!["10", "20", "30"]
            } else {
                vec!["10", "30"]
            }
        );
        assert_eq!(
            listed.total,
            if mode == Mode::Cloud { None } else { Some(4) }
        );
        assert_eq!(asset.attachments.len(), 1);
        let att = &asset.attachments[0];
        assert_eq!(att.id, "att557057");
        assert_eq!(att.title, "release-flow.png");
        assert_eq!(att.size, Some(4));
        assert_eq!(att.media_type.as_deref(), Some("image/png"));
        assert_eq!(att.source_revision.as_deref(), Some("2"));
        assert!(att.path.is_none());
        assert_eq!(att.not_downloaded.as_deref(), Some("not downloaded"));
        assert!(
            att.source_url
                .as_ref()
                .unwrap()
                .starts_with(&server.base_url())
        );
        assert!(!format!("{:?}", asset.fields).contains("private-account-id"));
        let requests = server.requests();
        assert!(
            requests
                .iter()
                .all(|request| request.method == "GET" && request.authorized)
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.target.contains("/download"))
        );
        if mode == Mode::DataCenter {
            assert_eq!(server.count(&path(&server, "/content/30")), 1);
        } else {
            assert_eq!(
                server.count(&path(&server, "/pages/31/ancestors")),
                0,
                "siblings reuse their parent chain"
            );
        }
    }
}

#[tokio::test]
async fn resolve_info_metadata_and_display_urls_use_native_api_and_exact_identity() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        assert!(
            matches!(provider.resolve_input("ENG").await.unwrap(),ProviderResolution::ConfluenceSpace {space_key} if space_key=="ENG")
        );
        assert!(server.requests().is_empty());
        let resolved = provider
            .resolve_input(&format!(
                "{}/display/ENG/Release+Checklist",
                server.base_url()
            ))
            .await
            .unwrap();
        let ProviderResolution::ConfluencePage(page) = resolved else {
            panic!("not a page")
        };
        assert_eq!(page.page_id, "30");
        assert_eq!(page.title, "Release Checklist");
        assert_eq!(page.space_key, "ENG");
        assert_eq!(
            page.canonical_url,
            confluence_page_url(&server.base_url(), "30")
        );
        assert_eq!(page.version, Some(7));
        let info_path = if mode == Mode::Cloud {
            "/pages/30"
        } else {
            "/content/30"
        };
        assert_eq!(
            server.count(&path(&server, info_path)),
            0,
            "display match is used without a redundant info request"
        );
        assert_eq!(provider.page_space("30").await.unwrap(), Some("ENG".into()));
        assert_eq!(provider.page_space("999").await.unwrap(), None);
        assert_eq!(
            provider
                .metadata(&request(&server, "30"))
                .await
                .unwrap()
                .title,
            "Release Checklist"
        );
        assert!(
            matches!(provider.resolve_input("30").await.unwrap(),ProviderResolution::ConfluencePage(page) if page.page_id=="30")
        );
        assert_eq!(
            provider
                .resolve_input(&format!("{}/display/ENG/Unknown", server.base_url()))
                .await
                .unwrap_err()
                .code,
            "source_not_found"
        );
    }
}

#[tokio::test]
async fn pagination_rebuilds_cursor_and_offset_requests_with_filters_and_reports_caps() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        server.state.lock().page_size = 1;
        let (provider, _) = provider(&server, true).await;
        let listing = provider
            .list_space_pages("ENG", 100, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(listing.complete);
        assert_eq!(listing.pages.len(), 4);
        let endpoint = if mode == Mode::Cloud {
            path(&server, "/spaces/99/pages")
        } else {
            path(&server, "/content/search")
        };
        let requests = server
            .requests()
            .into_iter()
            .filter(|r| r.target.split('?').next() == Some(endpoint.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 4);
        for (index, req) in requests.iter().enumerate() {
            let url = Url::parse(&format!(
                "{}{}",
                format!("http://127.0.0.1:{}", server.port),
                req.target
            ))
            .unwrap();
            let query = url
                .query_pairs()
                .into_owned()
                .collect::<std::collections::BTreeMap<_, _>>();
            if mode == Mode::Cloud {
                assert_eq!(query.get("depth").map(String::as_str), Some("all"));
                assert_eq!(query.get("status").map(String::as_str), Some("current"));
                assert_eq!(
                    query.get("cursor").map(String::as_str),
                    if index == 0 {
                        None
                    } else {
                        Some(match index {
                            1 => "1",
                            2 => "2",
                            _ => "3",
                        })
                    }
                );
            } else {
                assert_eq!(
                    query.get("cql").map(String::as_str),
                    Some("space=\"ENG\" and type=page")
                );
                assert_eq!(
                    query.get("expand").map(String::as_str),
                    Some("version,ancestors,space")
                );
                assert_eq!(query.get("start"), Some(&index.to_string()));
            }
        }
        let capped = provider
            .list_space_pages("ENG", 2, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(!capped.complete);
        assert_eq!(capped.pages.len(), 2);
        let exact = provider
            .list_space_pages("ENG", 4, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(exact.complete);
        let zero = provider
            .list_space_pages("ENG", 0, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(!zero.complete);
        assert!(zero.pages.is_empty());
    }
}

#[tokio::test]
async fn spaces_enumerate_completely_and_deterministically_in_both_deployments() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        {
            let mut state = server.state.lock();
            state.space_keys = vec!["ZED".into(), "ENG".into(), "AAA".into()];
            state.page_size = 1;
        }
        let (provider, _) = provider(&server, true).await;
        let spaces = provider.list_spaces().await.unwrap();
        assert_eq!(
            spaces
                .iter()
                .map(|space| space.key.as_str())
                .collect::<Vec<_>>(),
            vec!["AAA", "ENG", "ZED"]
        );
        assert_eq!(
            server.count(&path(
                &server,
                if mode == Mode::Cloud {
                    "/spaces"
                } else {
                    "/space"
                }
            )),
            3
        );
    }
}

#[tokio::test]
async fn continuations_never_follow_foreign_paths_origins_or_modified_filters() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        for bad in [
            "https://outside.example/wiki/api/v2/pages?cursor=2",
            "/unrelated?cursor=2",
            "/wiki/api/v2/spaces/99/pages?cursor=2&status=deleted",
        ] {
            let server = fixture(mode);
            server.state.lock().next_override = Some(bad.into());
            let (provider, _) = provider(&server, true).await;
            assert_eq!(
                provider
                    .list_space_pages("ENG", 100, &AtomicBool::new(false))
                    .await
                    .unwrap_err()
                    .code,
                "source_provider_contract"
            );
            assert!(
                !server
                    .requests()
                    .iter()
                    .any(|request| request.target.starts_with("/unrelated"))
            );
        }
    }
}

#[tokio::test]
async fn repeated_tokens_empty_continuations_and_offset_gaps_are_contract_errors() {
    for (mode, bad) in [
        (Mode::Cloud, "/wiki/api/v2/spaces/99/pages?cursor=1"),
        (
            Mode::DataCenter,
            "/confluence/rest/api/content/search?start=99",
        ),
    ] {
        let server = fixture(mode);
        {
            let mut state = server.state.lock();
            state.page_size = 1;
            state.next_override = Some(bad.into());
        }
        let (provider, _) = provider(&server, true).await;
        assert_eq!(
            provider
                .list_space_pages("ENG", 100, &AtomicBool::new(false))
                .await
                .unwrap_err()
                .code,
            "source_provider_contract"
        );
    }
    let server = fixture(Mode::Cloud);
    server.state.lock().overrides.insert(
        path(&server, "/spaces/99/pages"),
        Response::json(
            200,
            json!({"results":[],"_links":{"next":"/wiki/api/v2/spaces/99/pages?cursor=next"}}),
        ),
    );
    let (provider, _) = provider(&server, true).await;
    assert_eq!(
        provider
            .list_space_pages("ENG", 100, &AtomicBool::new(false))
            .await
            .unwrap_err()
            .code,
        "source_provider_contract"
    );
}

#[tokio::test]
async fn cancellation_between_http_calls_returns_an_incomplete_listing() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let cancel = Arc::new(AtomicBool::new(false));
        let endpoint = path(
            &server,
            if mode == Mode::Cloud {
                "/spaces/99/pages"
            } else {
                "/content/search"
            },
        );
        {
            let mut state = server.state.lock();
            state.page_size = 1;
            state.cancel_after_path = Some((endpoint.clone(), cancel.clone()));
        }
        let (provider, _) = provider(&server, true).await;
        let result = provider
            .list_space_pages("ENG", 100, &cancel)
            .await
            .unwrap();
        assert!(!result.complete);
        assert_eq!(server.count(&endpoint), 1);
        let already_cancelled = provider
            .list_space_pages("ENG", 100, &AtomicBool::new(true))
            .await
            .unwrap();
        assert!(!already_cancelled.complete);
        assert!(already_cancelled.pages.is_empty());
    }
}

#[tokio::test]
async fn labels_and_attachments_are_bounded_with_observable_partial_diagnostics() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        {
            let mut state = server.state.lock();
            state.labels_unbounded = true;
            state.attachments_unbounded = true;
            let page = state.pages.get_mut("30").unwrap();
            page.labels = (0..257).map(|n| format!("label-{n}")).collect();
            page.attachments = (0..257)
                .map(|n| {
                    (
                        format!("att{}", 1000 + n),
                        format!("file-{n}.png"),
                        "image/png".into(),
                        4,
                    )
                })
                .collect();
        }
        let (provider, _) = provider(&server, true).await;
        let asset = provider
            .fetch(&request(&server, "30"))
            .await
            .unwrap()
            .remove(0);
        assert_eq!(asset.attachments.len(), 256);
        assert!(
            asset
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "source_attachments_partial")
        );
        assert!(
            asset
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "source_labels_partial")
        );
        assert!(
            matches!(field(&asset,"labels"),Some(FrontmatterValue::Strings(labels)) if labels.len()==256)
        );
    }
}

#[tokio::test]
async fn attachment_metadata_pages_are_combined_until_the_cap_not_just_first_page() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        server.state.lock().pages.get_mut("30").unwrap().attachments = (0..256)
            .map(|n| {
                (
                    format!("att{}", 1000 + n),
                    format!("file-{n}.png"),
                    "image/png".into(),
                    4,
                )
            })
            .collect();
        let (provider, _) = provider(&server, true).await;
        let asset = provider
            .fetch(&request(&server, "30"))
            .await
            .unwrap()
            .remove(0);
        assert_eq!(asset.attachments.len(), 256);
        assert!(
            !asset
                .diagnostics
                .iter()
                .any(|d| d.code == "source_attachments_partial")
        );
        assert_eq!(
            server.count(&path(
                &server,
                if mode == Mode::Cloud {
                    "/pages/30/attachments"
                } else {
                    "/content/30/child/attachment"
                }
            )),
            2
        );
    }
}

#[tokio::test]
async fn missing_token_has_no_requests_and_replacing_token_recovers_without_profiles() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, credentials) = provider(&server, false).await;
        assert_eq!(
            provider.resolve_input("30").await.unwrap_err().code,
            "source_credential_required"
        );
        assert_eq!(
            provider
                .attachment_downloads("page")
                .await
                .unwrap_err()
                .code,
            "source_credential_required"
        );
        assert!(server.requests().is_empty());
        credentials
            .set(ProviderCredentialSetRequest {
                provider_id: "wiki".into(),
                kind: ProviderAuthKind::Bearer,
                username: None,
                token: TOKEN.into(),
            })
            .await
            .unwrap();
        provider.attachment_downloads("page").await.unwrap();
        provider.resolve_input("30").await.unwrap();
        assert!(server.requests().iter().all(|request| request.authorized));
        assert_eq!(
            provider
                .attachment_downloads("issue")
                .await
                .unwrap_err()
                .code,
            "source_capability_unavailable"
        );
        server.state.lock().unauthorized = true;
        let error = provider.resolve_input("30").await.unwrap_err();
        assert_eq!(error.code, "source_auth_failed");
        assert!(!error.message.contains(TOKEN));
    }
}

#[tokio::test]
async fn request_page_and_web_url_authority_mismatches_are_refused() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let mut invalid = request(&server, "30");
        invalid.authority.provider_instance = "https://outside.example/wiki".into();
        assert_eq!(
            provider.fetch(&invalid).await.unwrap_err().code,
            "source_identity_mismatch"
        );
        assert!(server.requests().is_empty());
        let endpoint = path(
            &server,
            if mode == Mode::Cloud {
                "/pages/30"
            } else {
                "/content/30"
            },
        );
        let mut value = page_json(server.state.lock().pages.get("30").unwrap(), mode);
        value["id"] = json!("31");
        server
            .state
            .lock()
            .overrides
            .insert(endpoint.clone(), Response::json(200, value));
        assert_eq!(
            provider.resolve_input("30").await.unwrap_err().code,
            "source_identity_mismatch"
        );
        let mut value = page_json(server.state.lock().pages.get("30").unwrap(), mode);
        value["_links"]["webui"] = json!("https://outside.example/wiki/spaces/ENG/pages/30");
        server
            .state
            .lock()
            .overrides
            .insert(endpoint, Response::json(200, value));
        assert_eq!(
            provider
                .fetch(&request(&server, "30"))
                .await
                .unwrap_err()
                .code,
            "source_identity_mismatch"
        );
    }
}

#[tokio::test]
async fn dc_non_page_content_is_capability_unavailable_and_stale_base_hints_are_ignored() {
    let server = fixture(Mode::DataCenter);
    let (provider, _) = provider(&server, true).await;
    let endpoint = path(&server, "/content/30");
    let mut value = page_json(
        server.state.lock().pages.get("30").unwrap(),
        Mode::DataCenter,
    );
    value["type"] = json!("blogpost");
    server
        .state
        .lock()
        .overrides
        .insert(endpoint.clone(), Response::json(200, value));
    assert_eq!(
        provider.resolve_input("30").await.unwrap_err().code,
        "source_capability_unavailable"
    );
    let mut value = page_json(
        server.state.lock().pages.get("30").unwrap(),
        Mode::DataCenter,
    );
    value["_links"]["base"] = json!("https://stale.example/confluence");
    server
        .state
        .lock()
        .overrides
        .insert(endpoint, Response::json(200, value));
    assert!(
        provider.fetch(&request(&server, "30")).await.is_ok(),
        "HTTP origin, not a legacy _links.base hint, proves the instance"
    );
}

#[tokio::test]
async fn cloud_user_display_name_ignores_only_403_or_404_and_never_persists_an_account_id() {
    for status in [200, 403, 404, 401, 500] {
        let server = fixture(Mode::Cloud);
        server.state.lock().user_status = status;
        let (provider, _) = provider(&server, true).await;
        let answer = provider.fetch(&request(&server, "30")).await;
        if [200, 403, 404].contains(&status) {
            let asset = answer.unwrap().remove(0);
            let expected = if status == 200 {
                Some(text("Fixture Author"))
            } else {
                None
            };
            assert_eq!(field(&asset, "last_modified_by"), expected.as_ref());
            assert!(!format!("{:?}", asset.fields).contains("private-account-id"));
        } else {
            assert_eq!(
                answer.unwrap_err().code,
                if status == 401 {
                    "source_auth_failed"
                } else {
                    "source_provider_failed"
                }
            );
        }
    }
}

#[tokio::test]
async fn cloud_folder_and_unknown_ancestor_types_preserve_all_ids_and_titles() {
    let server = fixture(Mode::Cloud);
    {
        let mut state = server.state.lock();
        state.pages.get_mut("30").unwrap().ancestors.insert(
            1,
            ("15".into(), "whiteboard".into(), "Ignored title".into()),
        );
    }
    let (provider, _) = provider(&server, true).await;
    let asset = provider
        .fetch(&request(&server, "30"))
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        field(&asset, "ancestor_ids"),
        Some(&strings(&["10", "15", "20"]))
    );
    assert_eq!(
        field(&asset, "ancestors"),
        Some(&strings(&["Engineering Home", "15", "Release Folder"]))
    );
}

#[tokio::test]
async fn large_storage_page_is_rendered_locally_without_a_smaller_input_limit() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        server.state.lock().pages.get_mut("30").unwrap().storage =
            format!("<p>{}</p>", "x".repeat(700_685));
        let (provider, _) = provider(&server, true).await;
        let asset = provider
            .fetch(&request(&server, "30"))
            .await
            .unwrap()
            .remove(0);
        assert_eq!(asset.body.len(), 700_685);
    }
}

#[tokio::test]
async fn attachment_downloads_address_ids_and_write_only_the_private_download_file() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let dest = Destination::new();
        let result = provider
            .download_attachment("30", &attachment(), &[], &dest.dir, &dest.path, budget(16))
            .await
            .unwrap();
        assert_eq!(result.attachment_id, "att557057");
        assert_eq!(result.file_name, "download");
        assert_eq!(dest.dir.read("download").unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(std::fs::read_dir(&dest.path).unwrap().count(), 1);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dest.path.join("download"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            server.count(&path(
                &server,
                if mode == Mode::Cloud {
                    "/attachments/att557057"
                } else {
                    "/content/att557057"
                }
            )),
            1
        );
        assert!(server.requests().iter().all(|request| request.authorized));
        assert!(
            server
                .requests()
                .iter()
                .any(|request| request.target.starts_with(&format!(
                    "{}/rest/api/content/30/child/attachment/att557057/download",
                    mode.context()
                )))
        );
    }
}

#[tokio::test]
async fn cloud_download_redirect_drops_authorization_on_the_media_origin() {
    let media = FakeConfluence::start(Mode::DataCenter);
    media
        .state
        .lock()
        .overrides
        .insert("/confluence/media".into(), Response::bytes(&[1, 2, 3, 4]));
    let server = fixture(Mode::Cloud);
    server.state.lock().download_redirect = Some(format!("{}/media", media.base_url()));
    let (provider, _) = provider(&server, true).await;
    let dest = Destination::new();
    provider
        .download_attachment("30", &attachment(), &[], &dest.dir, &dest.path, budget(16))
        .await
        .unwrap();
    assert_eq!(dest.dir.read("download").unwrap(), vec![1, 2, 3, 4]);
    assert!(server.requests().iter().all(|request| request.authorized));
    assert_eq!(media.requests().len(), 1);
    assert!(!media.requests()[0].authorization_present);
}

#[tokio::test]
async fn attachment_identity_and_size_mismatches_never_publish_a_file() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        for changed in ["page", "title", "id"] {
            let mut value = if mode == Mode::Cloud {
                json!({"id":"att557057","pageId":"30","title":"release-flow.png","fileSize":4,"downloadLink":"/rest/api/content/30/child/attachment/att557057/download"})
            } else {
                json!({"id":"att557057","type":"attachment","container":{"id":"30"},"title":"release-flow.png","extensions":{"fileSize":4},"_links":{"download":"/confluence/rest/api/content/30/child/attachment/att557057/download"}})
            };
            match changed {
                "page" => {
                    if mode == Mode::Cloud {
                        value["pageId"] = json!("31")
                    } else {
                        value["container"]["id"] = json!("31")
                    }
                }
                "title" => value["title"] = json!("other.png"),
                _ => value["id"] = json!("att1"),
            }
            let endpoint = path(
                &server,
                if mode == Mode::Cloud {
                    "/attachments/att557057"
                } else {
                    "/content/att557057"
                },
            );
            server
                .state
                .lock()
                .overrides
                .insert(endpoint, Response::json(200, value));
            let dest = Destination::new();
            assert_eq!(
                provider
                    .download_attachment(
                        "30",
                        &attachment(),
                        &[],
                        &dest.dir,
                        &dest.path,
                        budget(16)
                    )
                    .await
                    .unwrap_err()
                    .code,
                "source_identity_mismatch"
            );
            assert_eq!(std::fs::read_dir(&dest.path).unwrap().count(), 0);
        }
        server.state.lock().overrides.clear();
        server.state.lock().download_body = vec![1, 2, 3];
        let dest = Destination::new();
        assert_eq!(
            provider
                .download_attachment("30", &attachment(), &[], &dest.dir, &dest.path, budget(16))
                .await
                .unwrap_err()
                .code,
            "source_attachment_size"
        );
        assert_eq!(std::fs::read_dir(&dest.path).unwrap().count(), 0);
        server.state.lock().download_body = vec![1, 2, 3, 4];
        let dest = Destination::new();
        assert_eq!(
            provider
                .download_attachment("30", &attachment(), &[], &dest.dir, &dest.path, budget(3))
                .await
                .unwrap_err()
                .code,
            "source_attachment_size"
        );
        assert_eq!(std::fs::read_dir(&dest.path).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn dc_short_listing_without_a_next_link_is_incomplete_when_total_disagrees() {
    let server = fixture(Mode::DataCenter);
    let page = page_json(
        server.state.lock().pages.get("10").unwrap(),
        Mode::DataCenter,
    );
    server.state.lock().overrides.insert(
        path(&server, "/content/search"),
        Response::json(200, json!({"results":[page],"totalSize":4})),
    );
    let (provider, _) = provider(&server, true).await;
    let listing = provider
        .list_space_pages("ENG", 100, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(!listing.complete);
    assert_eq!(listing.total, Some(4));
    assert_eq!(listing.pages.len(), 1);
}

#[tokio::test]
async fn curated_rest_fixtures_normalize_through_the_same_http_consumer_boundary() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = FakeConfluence::start(mode);
        let parse = |text: &str| serde_json::from_str(text).unwrap();
        let (id, content, labels, attachments) = if mode == Mode::Cloud {
            (
                "123456789",
                parse(include_str!("fixtures/confluence/cloud/page/content.json")),
                parse(include_str!("fixtures/confluence/cloud/page/labels.json")),
                parse(include_str!(
                    "fixtures/confluence/cloud/page/attachments.json"
                )),
            )
        } else {
            (
                "524301",
                parse(include_str!("fixtures/confluence/dc/content.json")),
                parse(include_str!("fixtures/confluence/dc/labels.json")),
                parse(include_str!("fixtures/confluence/dc/attachments.json")),
            )
        };
        {
            let mut state = server.state.lock();
            if mode == Mode::Cloud {
                state.overrides.insert(
                    path(&server, "/pages/123456789"),
                    Response::json(200, content),
                );
                state.overrides.insert(
                    path(&server, "/pages/123456789/labels"),
                    Response::json(200, labels),
                );
                state.overrides.insert(
                    path(&server, "/pages/123456789/attachments"),
                    Response::json(200, attachments),
                );
                state.overrides.insert(path(&server,"/pages/123456789/ancestors"),Response::json(200,json!({"results":[{"id":"65601","type":"page"},{"id":"77001","type":"folder"},{"id":"98765","type":"page"}]})));
                state.overrides.insert(
                    path(&server, "/spaces/98314"),
                    Response::json(
                        200,
                        json!({"id":"98314","key":"SD","name":"Software Development"}),
                    ),
                );
                state.overrides.insert(path(&server,"/pages"),Response::json(200,json!({"results":[{"id":"65601","title":"Software Development Home"},{"id":"98765","title":"Engineering"}]})));
                state.overrides.insert(
                    path(&server, "/folders/77001"),
                    Response::json(200, json!({"id":"77001","title":"Runbooks"})),
                );
            } else {
                state.overrides.insert(
                    path(&server, "/content/524301"),
                    Response::json(200, content),
                );
                state.overrides.insert(
                    path(&server, "/content/524301/label"),
                    Response::json(200, labels),
                );
                state.overrides.insert(
                    path(&server, "/content/524301/child/attachment"),
                    Response::json(200, attachments),
                );
            }
        }
        let (provider, _) = provider(&server, true).await;
        let asset = provider
            .fetch(&request(&server, id))
            .await
            .unwrap()
            .remove(0);
        assert_eq!(asset.title, "Release Checklist");
        assert_eq!(
            asset.body,
            "# Release checklist\n\nCheck the deployment before shipping."
        );
        assert_eq!(asset.attachments.len(), 1);
        assert_eq!(asset.attachments[0].title, "release-flow.png");
        assert_eq!(asset.attachments[0].size, Some(2048));
        assert!(asset.diagnostics.is_empty());
        assert_eq!(
            field(&asset, "ancestor_ids"),
            Some(&strings(if mode == Mode::Cloud {
                &["65601", "77001", "98765"]
            } else {
                &["524289"]
            }))
        );
        assert_eq!(
            asset.source_revision.as_deref(),
            Some(if mode == Mode::Cloud { "7" } else { "12" })
        );
    }
}

#[tokio::test]
async fn dc_rest_search_captures_preserve_total_homepage_and_complete_ancestry() {
    let server = FakeConfluence::start(Mode::DataCenter);
    let mut first: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/confluence/dc/api-search-page-1.json"
    ))
    .unwrap();
    // The token alone is enough: the provider must rebuild the original CQL,
    // limit and expand rather than following this server-provided URL.
    first["_links"]["next"] = json!("/rest/api/content/search?start=2");
    let second = serde_json::from_str(include_str!(
        "fixtures/confluence/dc/api-search-page-2.json"
    ))
    .unwrap();
    let space =
        serde_json::from_str(include_str!("fixtures/confluence/dc/api-space-ENG.json")).unwrap();
    let mut second_url =
        Url::parse(&format!("{}/rest/api/content/search", server.base_url())).unwrap();
    second_url
        .query_pairs_mut()
        .append_pair("cql", "space=\"ENG\" and type=page")
        .append_pair("limit", "100")
        .append_pair("expand", "version,ancestors,space")
        .append_pair("start", "2");
    {
        let mut state = server.state.lock();
        state
            .overrides
            .insert(path(&server, "/space/ENG"), Response::json(200, space));
        state
            .overrides
            .insert(path(&server, "/content/search"), Response::json(200, first));
        state
            .overrides
            .insert(second_url.to_string(), Response::json(200, second));
    }
    let (provider, _) = provider(&server, true).await;
    let listing = provider
        .list_space_pages("ENG", 100, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(listing.complete);
    assert_eq!(listing.total, Some(3));
    assert_eq!(listing.homepage_id.as_deref(), Some("524289"));
    assert_eq!(listing.pages.len(), 3);
    assert_eq!(listing.pages[1].page_id, "524301");
    assert_eq!(listing.pages[1].ancestors, vec!["524289"]);
}

#[tokio::test]
async fn cancellation_also_stops_inside_cloud_ancestor_pagination() {
    let server = fixture(Mode::Cloud);
    let cancel = Arc::new(AtomicBool::new(false));
    let ancestors = path(&server, "/pages/30/ancestors");
    {
        let mut state = server.state.lock();
        state.cancel_after_path = Some((ancestors.clone(), cancel.clone()));
        state.overrides.insert(ancestors.clone(),Response::json(200,json!({"results":[{"id":"10","type":"page"}],"_links":{"next":"/wiki/api/v2/pages/30/ancestors?cursor=next"}})));
    }
    let (provider, _) = provider(&server, true).await;
    let result = provider
        .list_space_pages("ENG", 100, &cancel)
        .await
        .unwrap();
    assert!(!result.complete);
    assert_eq!(server.count(&ancestors), 1);
    assert_eq!(
        result
            .pages
            .iter()
            .map(|page| page.page_id.as_str())
            .collect::<Vec<_>>(),
        vec!["10"]
    );
}

#[tokio::test]
async fn cloud_listed_hierarchy_cycles_are_rejected_instead_of_building_self_ancestry() {
    let server = fixture(Mode::Cloud);
    server.state.lock().pages.get_mut("10").unwrap().ancestors =
        vec![("40".into(), "page".into(), "Child Page".into())];
    let (provider, _) = provider(&server, true).await;
    assert_eq!(
        provider
            .list_space_pages("ENG", 100, &AtomicBool::new(false))
            .await
            .unwrap_err()
            .code,
        "source_provider_contract"
    );
}

#[tokio::test]
async fn spaces_over_the_enumeration_cap_report_truncation_not_a_complete_subset() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = FakeConfluence::start(mode);
        {
            let mut state = server.state.lock();
            state.space_keys = (0..10_001).map(|index| format!("K{index}")).collect();
            state.page_size = 10_001;
        }
        let (provider, _) = provider(&server, true).await;
        assert_eq!(
            provider.list_spaces().await.unwrap_err().code,
            "source_truncated"
        );
    }
}

#[tokio::test]
async fn display_results_with_a_different_title_or_space_are_not_accepted() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let mut value = page_json(server.state.lock().pages.get("30").unwrap(), mode);
        value["title"] = json!("Different Page");
        server.state.lock().overrides.insert(
            path(
                &server,
                if mode == Mode::Cloud {
                    "/pages"
                } else {
                    "/content"
                },
            ),
            Response::json(200, json!({"results":[value]})),
        );
        let (provider, _) = provider(&server, true).await;
        assert_eq!(
            provider
                .resolve_input(&format!(
                    "{}/display/ENG/Release+Checklist",
                    server.base_url()
                ))
                .await
                .unwrap_err()
                .code,
            "source_not_found"
        );
    }
}

#[tokio::test]
async fn bounded_cql_delta_freezes_timezone_safe_envelope_and_only_requests_metadata() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let listing = provider.list_page_changes("ENG", 0, 60_000, 100, &AtomicBool::new(false)).await.unwrap();
        assert!(listing.complete);
        assert_eq!(listing.pages.len(), 4);
        assert_eq!(listing.pages.iter().find(|page| page.page_id == "40").unwrap().ancestors.last().map(String::as_str), Some("30"));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        for request in requests {
            let url = Url::parse(&format!("http://fixture{}", request.target)).unwrap();
            assert_eq!(url.path(), format!("{}/rest/api/content/search", mode.context()));
            let query = url.query_pairs().into_owned().collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(query["cql"], "type=page AND space=\"ENG\" AND lastmodified >= \"1969-12-31 10:00\" AND lastmodified < \"1970-01-01 14:01\" ORDER BY lastmodified ASC");
            assert_eq!(query["expand"], "version,ancestors,space");
            assert!(!query.contains_key("body-format"));
            assert!(request.authorized);
        }
    }
}

#[tokio::test]
async fn delta_caps_cancellation_totals_and_changed_filters_never_claim_completion() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let capped = provider.list_page_changes("ENG", 0, 60_000, 1, &AtomicBool::new(false)).await.unwrap();
        assert_eq!(capped.pages.len(), 1);
        assert!(!capped.complete);
        let cancelled = Arc::new(AtomicBool::new(false));
        let search_path = format!("{}/rest/api/content/search", mode.context());
        server.state.lock().cancel_after_path = Some((search_path.clone(), cancelled.clone()));
        let listing = provider.list_page_changes("ENG", 0, 60_000, 100, &cancelled).await.unwrap();
        assert!(!listing.complete);
        server.state.lock().cancel_after_path = None;
        server.state.lock().next_override = Some(format!("{search_path}?{}=2&cql=type%3Dpage",
            if mode == Mode::Cloud { "cursor" } else { "start" }));
        assert_eq!(provider.list_page_changes("ENG", 0, 60_000, 100, &AtomicBool::new(false))
            .await.unwrap_err().code, "source_provider_contract");
        server.state.lock().next_override = None;
        let page = server.state.lock().pages["10"].clone();
        server.state.lock().overrides.insert(search_path, Response::json(200, json!({
            "results":[page_json(&page, Mode::DataCenter)], "totalSize":2
        })));
        assert!(!provider.list_page_changes("ENG", 0, 60_000, 100, &AtomicBool::new(false)).await.unwrap().complete);
    }
}

#[tokio::test]
async fn standalone_versions_batch_deduplicate_and_preserve_hierarchy_without_body_fetches() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = FakeConfluence::start(mode);
        server.state.lock().page_size = 100;
        for n in 1..=251 {
            server.add_page(Page::new(&n.to_string(), &format!("Page {n}"), &[]));
        }
        let (batch_provider, _) = provider(&server, true).await;
        let mut ids = (1..=251).map(|n| n.to_string()).collect::<Vec<_>>();
        ids.push("1".into());
        ids.push("999".into()); // Completed enumeration may omit inaccessible IDs.
        let listing = batch_provider.page_versions(&ids, &AtomicBool::new(false)).await.unwrap();
        assert!(listing.complete);
        assert_eq!(listing.pages.len(), 251);
        assert!(listing.space_name.is_empty());
        let requests = server.requests();
        assert_eq!(requests.len(), if mode == Mode::Cloud { 2 } else { 3 });
        for request in requests {
            let url = Url::parse(&format!("http://fixture{}", request.target)).unwrap();
            let query = url.query_pairs().into_owned().collect::<std::collections::BTreeMap<_, _>>();
            if mode == Mode::Cloud {
                assert_eq!(url.path(), format!("{}/pages", mode.api()));
                assert!(query["id"].split(',').count() <= 250);
                assert!(!query.contains_key("body-format"));
            } else {
                assert!(query["cql"].starts_with("type=page AND id IN ("));
                assert_eq!(query["expand"], "version,ancestors,space");
                assert!(query["cql"].split(',').count() <= 100);
            }
        }
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let listing = provider.page_versions(&["40".into()], &AtomicBool::new(false)).await.unwrap();
        assert!(listing.complete);
        assert_eq!(listing.pages[0].ancestors.last().map(String::as_str), Some("30"));
        assert!(server.requests().iter().all(|request| !request.target.contains("body-format")));
    }
}

#[tokio::test]
async fn versions_and_delta_invalid_inputs_or_precancellation_are_bodyless_and_safe() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        for (lower, upper) in [(-1, 1), (1, 1), (2, 1)] {
            assert_eq!(provider.list_page_changes("ENG", lower, upper, 100, &AtomicBool::new(false))
                .await.unwrap_err().code, "source_provider_contract");
        }
        assert_eq!(provider.page_versions(&["../30".into()], &AtomicBool::new(false)).await.unwrap_err().code, "source_provider_contract");
        assert!(!provider.page_versions(&["30".into()], &AtomicBool::new(true)).await.unwrap().complete);
        assert!(!provider.list_page_changes("ENG", 0, 1, 100, &AtomicBool::new(true)).await.unwrap().complete);
        assert!(server.requests().is_empty());
    }
}

#[tokio::test]
async fn auxiliary_only_reads_discover_first_attachment_and_label_changes_without_body() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let first = provider.page_aux("31").await.unwrap();
        assert!(first.attachments.is_empty());
        assert!(first.labels_complete && first.attachments_complete);
        {
            let mut state = server.state.lock();
            let page = state.pages.get_mut("31").unwrap();
            page.labels.push("audit-added".into());
            page.attachments.push(("att22".into(), "new.png".into(), "image/png".into(), 4));
        }
        let second = provider.page_aux("31").await.unwrap();
        assert!(second.labels.contains(&"audit-added".into()));
        assert_eq!(second.attachments[0].id, "att22");
        assert!(second.labels_complete && second.attachments_complete);
        for request in server.requests() {
            let url = Url::parse(&format!("http://fixture{}", request.target)).unwrap();
            assert!(!url.query_pairs().any(|(key, value)| key == "body-format" || key == "expand" && value.contains("body")));
            assert!(!url.path().ends_with("/download"));
        }
        let page_path = path(&server, if mode == Mode::Cloud { "/pages/31" } else { "/content/31" });
        server.state.lock().overrides.insert(page_path, Response::json(200, json!({"id":"30","type":"page"})));
        assert_eq!(provider.page_aux("31").await.unwrap_err().code, "source_identity_mismatch");
    }
}

#[tokio::test]
async fn auxiliary_limits_are_truthful_and_cross_page_attachments_rejected() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        {
            let mut state = server.state.lock();
            let page = state.pages.get_mut("31").unwrap();
            page.labels = (0..257).map(|n| format!("label-{n}")).collect();
            page.attachments = (0..257).map(|n| (format!("att{n}"), format!("file{n}"), "text/plain".into(), 4)).collect();
        }
        let (provider, _) = provider(&server, true).await;
        let aux = provider.page_aux("31").await.unwrap();
        assert_eq!(aux.labels.len(), 256);
        assert_eq!(aux.attachments.len(), 256);
        assert!(!aux.labels_complete && !aux.attachments_complete);
        if mode == Mode::Cloud {
            server.state.lock().overrides.insert(path(&server, "/pages/31/attachments"), Response::json(200, json!({
                "results":[{"id":"att22","title":"wrong.png","pageId":"30"}]
            })));
            assert_eq!(provider.page_aux("31").await.unwrap_err().code, "source_identity_mismatch");
        }
    }
}

#[tokio::test]
async fn delta_rounds_fractional_bounds_outward_without_moving_query_time() {
    let server = fixture(Mode::Cloud);
    let (provider, _) = provider(&server, true).await;
    let listing = provider.list_page_changes("ENG", 59_999, 60_001, 100, &AtomicBool::new(false)).await.unwrap();
    assert!(listing.complete);
    for request in server.requests() {
        let url = Url::parse(&format!("http://fixture{}", request.target)).unwrap();
        let cql = url.query_pairs().find(|(key, _)| key == "cql").unwrap().1.into_owned();
        assert!(cql.contains("lastmodified >= \"1969-12-31 10:00\""));
        assert!(cql.contains("lastmodified < \"1970-01-01 14:02\""));
    }
}

#[tokio::test]
async fn standalone_versions_do_not_complete_cancelled_or_unexpected_id_responses() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (provider, _) = provider(&server, true).await;
        let endpoint = if mode == Mode::Cloud { path(&server, "/pages") }
            else { format!("{}/rest/api/content/search", mode.context()) };
        let cancellation = Arc::new(AtomicBool::new(false));
        server.state.lock().cancel_after_path = Some((endpoint.clone(), cancellation.clone()));
        assert!(!provider.page_versions(&["30".into()], &cancellation).await.unwrap().complete);
        server.state.lock().cancel_after_path = None;
        let unrelated = server.state.lock().pages["31"].clone();
        server.state.lock().overrides.insert(endpoint, Response::json(200, json!({
            "results":[page_json(&unrelated, mode)]
        })));
        assert_eq!(provider.page_versions(&["30".into()], &AtomicBool::new(false)).await.unwrap_err().code, "source_provider_contract");
    }
}

#[tokio::test]
async fn source_service_accepts_real_provider_metadata_listings_without_space_names() {
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = fixture(mode);
        let (http_provider, _) = provider(&server, true).await;
        let sources = SourceService::new(&configuration(&server), vec![Arc::new(http_provider)]).unwrap();
        let cancel = AtomicBool::new(false);

        let delta = sources.list_page_changes("wiki", "ENG", 0, 60_000, 100, &cancel).await.unwrap();
        assert!(delta.complete);
        assert!(delta.space_name.is_empty());
        assert_eq!(delta.pages.len(), 4);
        assert_eq!(delta.pages.iter().find(|page| page.page_id == "40").unwrap().ancestors.last().map(String::as_str), Some("30"));

        let versions = sources.page_versions("wiki", &["30".into(), "40".into()], &cancel).await.unwrap();
        assert!(versions.complete);
        assert!(versions.space_name.is_empty());
        assert_eq!(versions.pages.len(), 2);
        assert_eq!(versions.pages.iter().find(|page| page.page_id == "40").unwrap().ancestors.last().map(String::as_str), Some("30"));

        let aux = sources.page_aux("wiki", "31").await.unwrap();
        assert!(aux.labels_complete && aux.attachments_complete);
        assert!(aux.attachments.is_empty());

        let search = format!("{}/rest/api/content/search", mode.context());
        server.state.lock().overrides.insert(search, Response::json(200, json!({
            "results": [], "totalSize": 0
        })));
        let empty = sources.list_page_changes("wiki", "ENG", 60_000, 120_000, 100, &cancel).await.unwrap();
        assert!(empty.complete);
        assert!(empty.space_name.is_empty());
        assert!(empty.pages.is_empty());
        assert!(server.requests().iter().all(|request| !request.target.contains("body-format")));
    }
}
