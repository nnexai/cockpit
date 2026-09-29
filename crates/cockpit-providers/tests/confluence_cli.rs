//! Real confluence-cli harness against a loopback fixture server.
//!
//! Runs only with `COCKPIT_CONFLUENCE_CLI=<path to confluence>`; otherwise
//! each test prints `skipped: COCKPIT_CONFLUENCE_CLI unset`. The CLI reads a
//! private temporary profile (`authType: none`, `readOnly: true`) and never
//! the user's configuration; only argv is logged.
#![cfg(unix)]

#[path = "support/fake_confluence.rs"]
mod fake_confluence;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use cockpit_core::sources::{
    AttachmentRef, FrontmatterValue, ProviderResolution, SourceAuthority, SourceFetchRequest,
    SourceService,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
use cockpit_providers::confluence::allowlisted_argv;
use fake_confluence::{
    FakeConfluence, Mode, PROFILE, Page, attachment_payload,
};

fn cli() -> Option<PathBuf> {
    match std::env::var_os("COCKPIT_CONFLUENCE_CLI") {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => {
            println!("skipped: COCKPIT_CONFLUENCE_CLI unset");
            None
        }
    }
}

fn page(id: &str, title: &str, space_key: &str, version: u64) -> Page {
    Page {
        id: id.into(),
        title: title.into(),
        space_key: space_key.into(),
        space_name: format!("{space_key} Space"),
        version,
        storage: "<h2>Before the release</h2><ul><li>Freeze the branch</li><li>Run the smoke suite</li></ul>".into(),
        ancestors: vec![("65601".into(), "page".into(), "Home".into())],
        labels: vec!["release".into()],
        attachments: vec![("att101".into(), "release-flow.png".into(), "image/png".into(), 2048)],
    }
}

fn configuration(server: &FakeConfluence, login: &str) -> ProjectConfiguration {
    ProjectConfiguration {
        version: 1,
        repository_roots: Vec::new(),
        worktree_root: "/w".into(),
        companion_root: "/c".into(),
        state_root: "/s".into(),
        cache_root: "/cache".into(),
        library_root: "/l".into(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: vec![ProjectProvider {
            id: "wiki".into(),
            base_url: server.base_url(),
            executable: server.executable(),
            login: Some(login.into()),
        }],
        limits: ProjectLimits {
            catalog_depth: 1,
            catalog_entries: 1,
            git_timeout_ms: 5000,
            git_output_bytes: 1024 * 1024,
            operation_timeout_ms: 30_000,
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
    }
}

fn service(server: &FakeConfluence, login: &str) -> SourceService {
    let configuration = configuration(server, login);
    let providers = cockpit_providers::configured_providers(&configuration).unwrap();
    SourceService::new(&configuration, providers).unwrap()
}

fn request(server: &FakeConfluence, canonical_url: &str) -> SourceFetchRequest {
    let base_path = match server.mode {
        Mode::Cloud => "/wiki",
        Mode::DataCenter => "",
    };
    SourceFetchRequest {
        provider_id: "wiki".into(),
        artifact_url: canonical_url.into(),
        authority: SourceAuthority {
            provider_instance: server.base_url(),
            origin_host: "127.0.0.1".into(),
            origin_port: Some(server.port),
            origin_base_path: base_path.into(),
            owner: String::new(),
            repository: String::new(),
        },
    }
}

/// Every logged call carries the profile and passes the D16 allowlist; the
/// fixture server saw only GETs.
fn assert_read_only_boundary(server: &FakeConfluence, login: &str) {
    for argv in server.argv() {
        assert_eq!(argv[..2], ["--profile", login], "{argv:?}");
        let argv: Vec<OsString> = argv.iter().map(OsString::from).collect();
        allowlisted_argv(&argv).unwrap();
    }
    for request in server.requests() {
        assert!(request.starts_with("GET "), "{request}");
    }
}

fn calls(server: &FakeConfluence) -> Vec<String> {
    server
        .argv()
        .into_iter()
        .map(|argv| argv[2..].join(" "))
        .collect()
}

#[tokio::test]
async fn display_urls_resolve_through_real_find_including_titles_that_look_like_options() {
    let Some(cli) = cli() else { return };
    let server = FakeConfluence::start(Mode::DataCenter, &cli);
    server.add_page(page("524301", "Release Checklist", "ENG", 12));
    server.add_page(page("524399", "-draft notes", "ENG", 1));
    server.add_page(page("524400", "Release Checklist", "OPS", 3));
    let service = service(&server, PROFILE);
    let base = server.base_url();

    let resolution = service
        .resolve_input("wiki", &format!("{base}/display/ENG/Release+Checklist"))
        .await
        .unwrap();
    let ProviderResolution::ConfluencePage(found) = resolution else {
        panic!("expected a page");
    };
    assert_eq!(
        (found.page_id.as_str(), found.space_key.as_str()),
        ("524301", "ENG")
    );
    assert_eq!(
        found.source_url,
        format!("{base}/display/ENG/Release+Checklist")
    );
    assert_eq!(
        found.canonical_url,
        format!("{base}/pages/viewpage.action?pageId=524301")
    );

    let draft = service
        .resolve_input("wiki", &format!("{base}/display/ENG/-draft+notes"))
        .await
        .unwrap();
    assert!(matches!(draft, ProviderResolution::ConfluencePage(page) if page.page_id == "524399"));

    let missing = service
        .resolve_input("wiki", &format!("{base}/display/ENG/Nope"))
        .await
        .unwrap_err();
    assert_eq!(missing.code, "source_not_found");

    assert_eq!(
        calls(&server),
        [
            "find --space ENG --json -- Release Checklist",
            "info 524301 --json",
            "find --space ENG --json -- -draft notes",
            "info 524399 --json",
            "find --space ENG --json -- Nope",
        ]
    );
    assert_read_only_boundary(&server, PROFILE);
}

#[tokio::test]
async fn cloud_and_dc_pages_fetch_and_refresh_through_the_real_cli() {
    let Some(cli) = cli() else { return };
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = FakeConfluence::start(mode, &cli);
        let mut fixture = page("123456789", "Release Checklist", "SD", 7);
        if mode == Mode::Cloud {
            fixture
                .ancestors
                .push(("77001".into(), "folder".into(), "Runbooks".into()));
        }
        server.add_page(fixture);
        let service = service(&server, PROFILE);
        let ProviderResolution::ConfluencePage(found) =
            service.resolve_input("wiki", "123456789").await.unwrap()
        else {
            panic!("expected a page");
        };
        let fetched = service
            .fetch_assets(request(&server, &found.canonical_url), false)
            .await
            .unwrap();
        let asset = &fetched.assets[0];
        assert_eq!(asset.source.resource_type, "page", "{mode:?}");
        assert_eq!(asset.source.canonical_id, "123456789");
        assert_eq!(asset.source_revision.as_deref(), Some("7"));
        assert_eq!(asset.source_url.as_deref(), Some(found.source_url.as_str()));
        assert!(
            asset.body.contains("Freeze the branch"),
            "{mode:?}: {}",
            asset.body
        );
        assert_eq!(asset.container.as_ref().unwrap().label, "SD · SD Space");
        let field = |key: &str| {
            asset
                .fields
                .iter()
                .find(|field| field.key == key)
                .map(|field| field.value.clone())
        };
        assert_eq!(
            field("labels"),
            Some(FrontmatterValue::Strings(vec!["release".into()]))
        );
        assert_eq!(
            field("last_modified_by"),
            Some(FrontmatterValue::String("Fixture Author".into()))
        );
        assert_eq!(asset.attachments.len(), 1);
        assert_eq!(asset.attachments[0].size, Some(2048));
        let serialized = serde_json::to_string(asset).unwrap();
        assert!(!serialized.contains("<email>") && !serialized.contains("<account-id>"));

        server.update_page("123456789", |page| {
            page.version = 8;
            page.storage = "<p>Ship it after sign-off.</p>".into();
        });
        let refreshed = service
            .fetch_assets(request(&server, &found.canonical_url), false)
            .await
            .unwrap();
        assert_eq!(refreshed.assets[0].source_revision.as_deref(), Some("8"));
        assert!(refreshed.assets[0].body.contains("Ship it after sign-off."));
        assert_ne!(
            cockpit_core::sources::content_revision(asset),
            cockpit_core::sources::content_revision(&refreshed.assets[0])
        );
        assert_read_only_boundary(&server, PROFILE);
    }
}


#[tokio::test]
async fn cloud_and_dc_attachment_downloads_are_id_mapped_and_confined() {
    let Some(cli) = cli() else { return };
    let hostile = [
        ("att201", "../x"),
        ("att202", "a/b.png"),
        ("att203", "con"),
        ("att204", "-rf.png"),
        ("att205", ".hidden"),
        ("att206", " pad.txt "),
        ("att207", "x*y.png"),
        ("att208", "xzy.png"),
        ("att209", "Report.PDF"),
        ("att210", "report.pdf"),
    ];
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = FakeConfluence::start(mode, &cli);
        let mut fixture = page("123456789", "Attachment Safety", "SD", 7);
        fixture.attachments = hostile
            .iter()
            .map(|(id, title)| ((*id).into(), (*title).into(), "application/octet-stream".into(), 64))
            .collect();
        server.add_page(fixture);
        let config = configuration(&server, PROFILE);
        let providers = cockpit_providers::configured_providers(&config).unwrap();
        let provider = providers.iter().find(|provider| provider.provider_id() == "wiki").unwrap();
        let download_root = server.root.join("private-staging");
        fs::create_dir_all(&download_root).unwrap();
        let siblings: Vec<AttachmentRef> = hostile
            .iter()
            .map(|(id, title)| AttachmentRef {
                id: (*id).into(),
                title: (*title).into(),
                bytes: Some(64),
            })
            .collect();
        for (index, (id, title)) in hostile.iter().enumerate() {
            let dest_path = download_root.join(format!("dl-{index}"));
            fs::create_dir(&dest_path).unwrap();
            let dest = Dir::open_ambient_dir(&dest_path, ambient_authority()).unwrap();
            let attachment = AttachmentRef {
                id: (*id).into(),
                title: (*title).into(),
                bytes: Some(64),
            };
            let siblings: Vec<_> = siblings
                .iter()
                .filter(|sibling| sibling.id != *id)
                .cloned()
                .collect();
            let pattern = cockpit_core::sources::confluence_attachment_pattern(title);
            let max_files = 1 + siblings.iter().filter(|sibling| cockpit_core::sources::confluence_glob_matches(&pattern, &sibling.title)).count();
            let budget = cockpit_core::process::StagingBudget { bytes: 64 * max_files as u64, max_files };
            assert_eq!(dest.entries().unwrap().count(), 0);
            let downloaded = provider
                .download_attachment("123456789", &attachment, &siblings, &dest, &dest_path, budget)
                .await
                .unwrap_or_else(|error| panic!("{mode:?} {title:?}: {error:?}; argv={:?}; requests={:?}", calls(&server), server.requests()));
            assert_eq!(downloaded.attachment_id, *id);
            assert_eq!(PathBuf::from(&downloaded.file_name).components().count(), 1);
            assert_eq!(
                fs::read(dest_path.join(&downloaded.file_name)).unwrap(),
                attachment_payload(id, 64),
                "{mode:?} {title:?}"
            );
            let files: Vec<_> = fs::read_dir(&dest_path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(files, [std::ffi::OsString::from(&downloaded.file_name)]);
        }
        let download_calls = calls(&server);
        assert_eq!(
            download_calls
                .iter()
                .filter(|call| call.contains("attachments 123456789 --download"))
                .count(),
            hostile.len(),
            "{mode:?}"
        );
        let staging_entries: Vec<_> = fs::read_dir(&download_root)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        assert_eq!(staging_entries.len(), hostile.len());
        assert!(staging_entries
            .iter()
            .all(|entry| entry.file_type().unwrap().is_dir()));
        assert_read_only_boundary(&server, PROFILE);
        assert!(server.requests().iter().all(|request| request.starts_with("GET ")));
    }
}
#[tokio::test]
async fn real_cli_identity_auth_and_profile_failures_map_to_stable_codes() {
    let Some(cli) = cli() else { return };
    let server = FakeConfluence::start(Mode::Cloud, &cli);
    server.add_page(page("123456789", "Release Checklist", "SD", 7));

    let not_found = service(&server, PROFILE)
        .resolve_input("wiki", "1")
        .await
        .unwrap_err();
    assert_eq!(not_found.code, "source_not_found");

    let missing_profile = service(&server, "cockpit-missing-profile")
        .resolve_input("wiki", "123456789")
        .await
        .unwrap_err();
    assert_eq!(missing_profile.code, "source_auth_failed");

    server.state.lock().unwrap().links_base = Some("http://evil.test/wiki".into());
    let mismatch = service(&server, PROFILE)
        .resolve_input("wiki", "123456789")
        .await
        .unwrap_err();
    assert_eq!(mismatch.code, "source_identity_mismatch");

    {
        let mut state = server.state.lock().unwrap();
        state.links_base = None;
        state.unauthorized = true;
    }
    let auth = service(&server, PROFILE)
        .resolve_input("wiki", "123456789")
        .await
        .unwrap_err();
    assert_eq!(auth.code, "source_auth_failed");
    for argv in server.argv() {
        let argv: Vec<OsString> = argv.iter().map(OsString::from).collect();
        allowlisted_argv(&argv).unwrap();
    }
}

#[tokio::test]
async fn cloud_and_dc_space_enumeration_and_paged_listing_use_only_gets() {
    let Some(cli) = cli() else { return };
    for mode in [Mode::Cloud, Mode::DataCenter] {
        let server = FakeConfluence::start(mode, &cli);
        let mut home = page("100", "Home", "SD", 4);
        home.ancestors.clear();
        server.add_page(home);
        let mut top = page("101", "Top level", "SD", 2);
        top.ancestors.clear();
        server.add_page(top);
        let mut child = page("102", "Nested child", "SD", 7);
        child.ancestors = vec![
            ("100".into(), "page".into(), "Home".into()),
            ("150".into(), "folder".into(), "Runbooks".into()),
        ];
        server.add_page(child);
        let other = page("201", "Other space", "ENG", 1);
        server.add_page(other);

        let configuration = configuration(&server, PROFILE);
        let providers = cockpit_providers::configured_providers(&configuration).unwrap();
        let provider = providers.iter().find(|provider| provider.provider_id() == "wiki").unwrap();
        let spaces = provider.list_spaces().await.unwrap();
        assert!(spaces.iter().any(|space| space.key == "SD" && space.name == "SD Space"), "{spaces:?}");
        let listing = provider.list_space_pages("SD", 100, &std::sync::atomic::AtomicBool::new(false))
            .await.unwrap();
        assert!(listing.complete, "{mode:?}: {listing:?}");
        assert_eq!(listing.total, Some(3), "{mode:?}");
        assert_eq!(listing.homepage_id.as_deref(), Some("100"));
        let by_id: std::collections::BTreeMap<_, _> = listing.pages.iter()
            .map(|page| (page.page_id.as_str(), page)).collect();
        assert_eq!(by_id.len(), 3);
        assert_eq!(by_id["102"].ancestors, vec!["100".to_owned(), "150".to_owned()]);
        assert_eq!(by_id["102"].version, 7);
        assert_eq!(by_id["102"].position, Some(2));
        assert_eq!(provider.page_space("102").await.unwrap().as_deref(), Some("SD"));
        assert_eq!(provider.page_space("999").await.unwrap(), None);
        let partial = provider.list_space_pages(
            "SD", 2, &std::sync::atomic::AtomicBool::new(false),
        ).await.unwrap();
        assert!(!partial.complete, "{mode:?}: {partial:?}");
        assert_eq!(partial.pages.len(), 2);
        assert_eq!(partial.total, Some(3));
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let cancelled_listing = provider.list_space_pages("SD", 100, &cancelled).await.unwrap();
        assert!(!cancelled_listing.complete);
        assert!(cancelled_listing.pages.is_empty());
        assert_read_only_boundary(&server, PROFILE);
        let requests = server.requests();
        assert!(requests.iter().any(|request| request.contains("/space?")), "{requests:?}");
        assert!(requests.iter().any(|request| request.contains("/content/search?") && request.contains("cql=")), "{requests:?}");
        assert!(requests.iter().all(|request| !request.contains("/download/")), "{requests:?}");
        let calls = calls(&server);
        assert!(calls.iter().all(|call| !call.contains("--download")), "{calls:?}");
    }
}
