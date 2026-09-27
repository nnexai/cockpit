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
use std::path::PathBuf;

use cockpit_core::sources::{
    FrontmatterValue, ProviderResolution, SourceAuthority, SourceFetchRequest, SourceService,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
use cockpit_providers::confluence::allowlisted_argv;
use fake_confluence::{FakeConfluence, Mode, PROFILE, Page};

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
