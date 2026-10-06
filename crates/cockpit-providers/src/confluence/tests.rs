use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cockpit_core::credentials::{MemoryVault, ProviderCredential, ProviderCredentials};
use cockpit_core::sources::{
    FrontmatterValue, ProviderResolution, SourceAuthority, SourceFetchRequest, SourceProvider,
    confluence_attachment_pattern, confluence_glob_matches,
};
use cockpit_protocol::credentials::{ProviderAuthKind, ProviderCredentialSetRequest};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
use serde_json::{Value, json};
use url::Url;

use super::{
    ConfluenceApi, ConfluenceCall, ConfluenceInput, ConfluenceSourceProvider, ContentExpand,
    SearchPage, allowlisted_argv, classify_failure, confluence_args, inject_credential,
    parse_confluence_input,
};
const CLOUD: &str = "https://nnexai.atlassian.net/wiki";
/// The stored fixture token; the shim reports only whether it received it.
const TOKEN: &str = "cockpit-fixture-token-7f3a";
const DC: &str = "https://confluence.example.com/confluence";

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!("../../tests/fixtures/confluence/", $path))
    };
}

fn strings(argv: &[OsString]) -> Vec<&str> {
    argv.iter().map(|arg| arg.to_str().unwrap()).collect()
}

fn os(argv: &[&str]) -> Vec<OsString> {
    argv.iter().map(OsString::from).collect()
}

fn page(id: &str) -> String {
    id.into()
}

#[test]
fn confluence_args_render_exactly_the_allowlisted_table() {
    let dest = PathBuf::from("/lib/.cockpit/staging/op/dl-1");
    let every = ContentExpand::ALL.to_vec();
    let table: Vec<(ConfluenceCall, Vec<&str>)> = vec![
        (ConfluenceCall::Spaces, vec!["spaces", "--all", "--json"]),
        (
            ConfluenceCall::Info {
                page_id: page("123"),
            },
            vec!["info", "123", "--json"],
        ),
        (
            ConfluenceCall::Read {
                page_id: page("123"),
            },
            vec!["read", "123", "--format", "markdown"],
        ),
        (
            ConfluenceCall::Find {
                space_key: "ENG".into(),
                title: "-draft Release".into(),
            },
            vec!["find", "--space", "ENG", "--json", "--", "-draft Release"],
        ),
        (
            ConfluenceCall::Attachments {
                page_id: page("123"),
            },
            vec!["attachments", "123", "--json"],
        ),
        (
            ConfluenceCall::DownloadAttachment {
                page_id: page("123"),
                pattern: "-rf?.png".into(),
                dest: dest.clone(),
            },
            vec![
                "attachments",
                "123",
                "--download",
                "--dest",
                "/lib/.cockpit/staging/op/dl-1",
                "--pattern=-rf?.png",
                "--json",
            ],
        ),
        (
            ConfluenceCall::Api(ConfluenceApi::Content {
                page_id: page("123"),
                expand: every.clone(),
            }),
            vec![
                "api",
                "content/123",
                "-X",
                "GET",
                "-f",
                "expand=ancestors,version,space,history.lastUpdated,metadata.labels",
            ],
        ),
        (
            ConfluenceCall::Api(ConfluenceApi::Labels {
                page_id: page("123"),
            }),
            vec!["api", "content/123/label", "-X", "GET"],
        ),
        (
            ConfluenceCall::Api(ConfluenceApi::Space {
                space_key: "~5af4".into(),
            }),
            vec!["api", "space/~5af4", "-X", "GET", "-f", "expand=homepage"],
        ),
        (
            ConfluenceCall::Api(ConfluenceApi::Search {
                space_key: "SD".into(),
                limit: 100,
                expand: vec![
                    ContentExpand::Version,
                    ContentExpand::Ancestors,
                    ContentExpand::Space,
                ],
                page: None,
            }),
            vec![
                "api",
                "content/search",
                "-X",
                "GET",
                "-f",
                "cql=space=\"SD\" and type=page",
                "-f",
                "limit=100",
                "-f",
                "expand=version,ancestors,space",
            ],
        ),
        (
            ConfluenceCall::Api(ConfluenceApi::Search {
                space_key: "SD".into(),
                limit: 1,
                expand: vec![ContentExpand::Version],
                page: Some(SearchPage::Cursor("raNDoMsTRiNg%3D%3D".into())),
            }),
            vec![
                "api",
                "content/search",
                "-X",
                "GET",
                "-f",
                "cql=space=\"SD\" and type=page",
                "-f",
                "limit=1",
                "-f",
                "expand=version",
                "-f",
                "cursor=raNDoMsTRiNg%3D%3D",
            ],
        ),
        (
            ConfluenceCall::Api(ConfluenceApi::Search {
                space_key: "ENG".into(),
                limit: 2,
                expand: vec![ContentExpand::Version],
                page: Some(SearchPage::Start(2)),
            }),
            vec![
                "api",
                "content/search",
                "-X",
                "GET",
                "-f",
                "cql=space=\"ENG\" and type=page",
                "-f",
                "limit=2",
                "-f",
                "expand=version",
                "-f",
                "start=2",
            ],
        ),
    ];
    for (call, expected) in table {
        let argv = confluence_args(&call).unwrap();
        assert_eq!(strings(&argv), expected, "{call:?}");
        let mut full = os(&["--profile", "cockpit-readonly"]);
        full.extend(argv);
        allowlisted_argv(&full).unwrap_or_else(|error| panic!("{call:?}: {}", error.message));
        assert!(
            !full.iter().any(|arg| {
                let arg = arg.to_str().unwrap();
                [
                    "--token", "--email", "--cookie", "--input", "-H", "-i", "--jq",
                ]
                .iter()
                .any(|flag| arg == *flag || arg.starts_with(&format!("{flag}=")))
            }),
            "{call:?}"
        );
    }
}

#[test]
fn confluence_args_refuse_malformed_values_before_any_process() {
    let dest = PathBuf::from("/lib/staging/dl-1");
    let refused = [
        ConfluenceCall::Info {
            page_id: "12a".into(),
        },
        ConfluenceCall::Info { page_id: "".into() },
        ConfluenceCall::Read {
            page_id: "123456789012345678901".into(),
        },
        ConfluenceCall::Attachments {
            page_id: "-1".into(),
        },
        ConfluenceCall::Find {
            space_key: "EN\"G".into(),
            title: "x".into(),
        },
        ConfluenceCall::Find {
            space_key: "ENG".into(),
            title: "".into(),
        },
        ConfluenceCall::Find {
            space_key: "ENG".into(),
            title: "line\nbreak".into(),
        },
        ConfluenceCall::Find {
            space_key: "ENG".into(),
            title: "x".repeat(256),
        },
        ConfluenceCall::DownloadAttachment {
            page_id: "1".into(),
            pattern: "*.png".into(),
            dest: dest.clone(),
        },
        ConfluenceCall::DownloadAttachment {
            page_id: "1".into(),
            pattern: " a.png".into(),
            dest: dest.clone(),
        },
        ConfluenceCall::DownloadAttachment {
            page_id: "1".into(),
            pattern: "a.png".into(),
            dest: PathBuf::from("relative/dl"),
        },
        ConfluenceCall::DownloadAttachment {
            page_id: "1".into(),
            pattern: "a.png".into(),
            dest: PathBuf::from("/lib/../etc"),
        },
        ConfluenceCall::Api(ConfluenceApi::Content {
            page_id: "1".into(),
            expand: vec![],
        }),
        ConfluenceCall::Api(ConfluenceApi::Content {
            page_id: "1".into(),
            expand: vec![ContentExpand::Version, ContentExpand::Version],
        }),
        ConfluenceCall::Api(ConfluenceApi::Space {
            space_key: "SD/../content".into(),
        }),
        ConfluenceCall::Api(ConfluenceApi::Search {
            space_key: "SD".into(),
            limit: 0,
            expand: vec![ContentExpand::Version],
            page: None,
        }),
        ConfluenceCall::Api(ConfluenceApi::Search {
            space_key: "SD".into(),
            limit: 101,
            expand: vec![ContentExpand::Version],
            page: None,
        }),
        ConfluenceCall::Api(ConfluenceApi::Search {
            space_key: "SD".into(),
            limit: 10,
            expand: vec![ContentExpand::Version],
            page: Some(SearchPage::Cursor("a&cql=type=blogpost".into())),
        }),
    ];
    for call in refused {
        assert_eq!(
            confluence_args(&call).unwrap_err().code,
            "source_provider_contract",
            "{call:?}"
        );
    }
}

#[test]
fn attachment_patterns_keep_hostile_titles_literal_and_predict_cli_overmatches() {
    let cases = [
        ("../x", "../x"),
        ("a/b.png", "a/b.png"),
        ("con", "con"),
        ("-rf.png", "-rf.png"),
        (".hidden", ".hidden"),
        (" pad.txt ", "?pad.txt?"),
        ("x*y.png", "x?y.png"),
        ("x?y.png", "x?y.png"),
    ];
    for (title, expected) in cases {
        assert_eq!(confluence_attachment_pattern(title), expected, "{title:?}");
    }
    let star = confluence_attachment_pattern("x*y.png");
    assert!(confluence_glob_matches(&star, "xzy.png"));
    assert!(!confluence_glob_matches(&star, "xy.png"));
    let pdf = confluence_attachment_pattern("Report.PDF");
    assert!(confluence_glob_matches(&pdf, "report.pdf"));
    assert!(!confluence_glob_matches(&pdf, "Report.PDFx"));
    assert_eq!(confluence_attachment_pattern("  "), "??");
}

#[test]
fn allowlist_refuses_every_argv_outside_the_table() {
    let hostile: &[&[&str]] = &[
        &["info", "1", "--json"],
        &["--profile", "-x", "info", "1", "--json"],
        &["--profile", "p", "create", "Title", "SD"],
        &["--profile", "p", "update", "1", "--content", "x"],
        &["--profile", "p", "delete", "1", "--yes"],
        &["--profile", "p", "children", "1", "--recursive", "--json"],
        &["--profile", "p", "init", "--token", "secret"],
        &["--profile", "p", "info", "1", "--json", "--token", "t"],
        &["--profile", "p", "info", "1", "--json", "--email", "e@x"],
        &[
            "--profile",
            "p",
            "info",
            "1",
            "--json",
            "--cookie",
            "JSESSIONID=x",
        ],
        &["--profile", "p", "info", "1a", "--json"],
        &[
            "--profile",
            "p",
            "info",
            "https://evil.test/wiki/pages/1",
            "--json",
        ],
        &["--profile", "p", "read", "1", "--format", "storage"],
        &["--profile", "p", "find", "--space", "SD", "--json", "Title"],
        &[
            "--profile",
            "p",
            "find",
            "--space",
            "SD",
            "--json",
            "--",
            "a\u{7}b",
        ],
        &[
            "--profile",
            "p",
            "attachments",
            "1",
            "--download",
            "--dest",
            "rel",
            "--pattern=a",
            "--json",
        ],
        &[
            "--profile",
            "p",
            "attachments",
            "1",
            "--download",
            "--dest",
            "/d",
            "--pattern",
            "a",
            "--json",
        ],
        &[
            "--profile",
            "p",
            "attachments",
            "1",
            "--download",
            "--dest",
            "/d",
            "--pattern=*",
            "--json",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1",
            "-X",
            "POST",
            "-f",
            "expand=version",
        ],
        &["--profile", "p", "api", "content/1", "-f", "expand=version"],
        &["--profile", "p", "api", "content/1/label"],
        &[
            "--profile",
            "p",
            "api",
            "content/1",
            "-X",
            "GET",
            "-X",
            "GET",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "https://evil.test/rest/api/content/1",
            "-X",
            "GET",
        ],
        &[
            "--profile",
            "p",
            "api",
            "/wiki/rest/api/content/1",
            "-X",
            "GET",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1",
            "-X",
            "GET",
            "--input",
            "body.json",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1/label",
            "-X",
            "GET",
            "-H",
            "Authorization: x",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1/label",
            "-X",
            "GET",
            "-i",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1/label",
            "-X",
            "GET",
            "--jq",
            ".x",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1/label",
            "-X",
            "GET",
            "--silent",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1",
            "-X",
            "GET",
            "-f",
            "expand=body.storage",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/1/child/attachment",
            "-X",
            "GET",
        ],
        &[
            "--profile",
            "p",
            "api",
            "space/SD",
            "-X",
            "GET",
            "-f",
            "expand=permissions",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=space=\"SD\" or type=page",
            "-f",
            "limit=2",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=space=\"SD\" and type=page and creator=x",
            "-f",
            "limit=2",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=type=page",
            "-f",
            "limit=2",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=space=\"SD\" and type=page",
            "-f",
            "limit=200",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=space=\"SD\" and type=page",
            "-f",
            "limit=02",
            "-f",
            "expand=version",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=space=\"SD\" and type=page",
            "-f",
            "limit=2",
            "-f",
            "expand=version",
            "-f",
            "cursor=a",
            "-f",
            "start=2",
        ],
        &[
            "--profile",
            "p",
            "api",
            "content/search",
            "-X",
            "GET",
            "-f",
            "cql=space=\"SD\" and type=page",
            "-f",
            "limit=2",
            "-f",
            "expand=version",
            "-f",
            "next=https://evil.test",
        ],
    ];
    for argv in hostile {
        assert_eq!(
            allowlisted_argv(&os(argv)).unwrap_err().code,
            "source_provider_contract",
            "{argv:?}"
        );
    }
}

#[test]
fn confluence_input_recognizes_cloud_and_dc_page_and_space_shapes() {
    let cloud = Url::parse(CLOUD).unwrap();
    let dc = Url::parse(DC).unwrap();
    let page = |id: &str| ConfluenceInput::Page { page_id: id.into() };
    let space = |key: &str| ConfluenceInput::Space {
        space_key: key.into(),
    };
    let display = |key: &str, title: &str| ConfluenceInput::Display {
        space_key: key.into(),
        title: title.into(),
    };
    let cases = [
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/spaces/SD/pages/123456789/Release+Checklist",
            page("123456789"),
        ),
        (
            &cloud,
            "https://NNEXAI.atlassian.net/wiki/spaces/SD/pages/123456789",
            page("123456789"),
        ),
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/pages/viewpage.action?pageId=42",
            page("42"),
        ),
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/spaces/SD",
            space("SD"),
        ),
        (
            &cloud,
            "https://nnexai.atlassian.net/wiki/spaces/~5af4129c/overview",
            space("~5af4129c"),
        ),
        (&cloud, " 123456789 ", page("123456789")),
        (&cloud, "SD", space("SD")),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/Release+Checklist",
            display("ENG", "Release Checklist"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/Parent/Child%20Page",
            display("ENG", "Child Page"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/-draft+notes",
            display("ENG", "-draft notes"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/a%2Bb",
            display("ENG", "a+b"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG",
            space("ENG"),
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=524301",
            page("524301"),
        ),
    ];
    for (base, input, expected) in cases {
        assert_eq!(
            parse_confluence_input(base, input).unwrap(),
            expected,
            "{input}"
        );
    }
    let rejected = [
        (
            &dc,
            "https://confluence.example.com/confluence/x/DQAI",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=1&pageId=2",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=1a",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/%FF",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/confluence/display/ENG/a%0Ab",
            "library_input_unrecognized",
        ),
        (
            &dc,
            "https://confluence.example.com/other/display/ENG/X",
            "source_identity_mismatch",
        ),
        (
            &dc,
            "http://confluence.example.com/confluence/display/ENG/X",
            "source_identity_mismatch",
        ),
        (
            &dc,
            "https://confluence.example.com:8443/confluence/display/ENG/X",
            "source_identity_mismatch",
        ),
        (
            &cloud,
            "https://user:pw@nnexai.atlassian.net/wiki/spaces/SD",
            "source_identity_mismatch",
        ),
        (
            &cloud,
            "https://evil.atlassian.net/wiki/spaces/SD/pages/1",
            "source_identity_mismatch",
        ),
        (
            &cloud,
            "123456789012345678901",
            "library_input_unrecognized",
        ),
        (&cloud, "release notes", "library_input_unrecognized"),
        (
            &cloud,
            "ftp://nnexai.atlassian.net/wiki/spaces/SD",
            "library_input_unrecognized",
        ),
    ];
    for (base, input, code) in rejected {
        assert_eq!(
            parse_confluence_input(base, input).unwrap_err().code,
            code,
            "{input}"
        );
    }
}

#[test]
fn confluence_failures_map_to_stable_codes_without_echoing_output() {
    let cases = [
        (
            fixture!("cloud/info-not-found.stderr.txt"),
            "source_not_found",
        ),
        (
            fixture!("cloud/attachments-not-found.stderr.txt"),
            "source_not_found",
        ),
        (
            fixture!("cloud/api-content-not-found.stderr.txt"),
            "source_not_found",
        ),
        (
            fixture!("cloud/labels-not-found.stderr.txt"),
            "source_not_found",
        ),
        (
            fixture!("cloud/read-not-found.stderr.txt"),
            "source_not_found",
        ),
        (
            fixture!("cloud/find-release-checklist.stderr.txt"),
            "source_not_found",
        ),
        (
            fixture!("cloud/info-bogus-profile.stderr.txt"),
            "source_auth_failed",
        ),
        (
            fixture!("cloud/page/info-auth-failed.stderr.json"),
            "source_auth_failed",
        ),
        (
            fixture!("dc/info-auth-failed.stderr.json"),
            "source_auth_failed",
        ),
        (
            "{\"code\":401,\"message\":\"Unauthorized; scope does not match\"}",
            "source_auth_failed",
        ),
        (
            "Error: Authentication failed (401 Unauthorized).\nPlease verify",
            "source_auth_failed",
        ),
        ("❌ Profile \"missing\" not found!\n", "source_auth_failed"),
        (
            "{\"error\":\"connect ECONNREFUSED\",\"code\":\"NETWORK\",\"status\":null}",
            "source_provider_failed",
        ),
        ("boom secret-token-value", "source_provider_failed"),
    ];
    for (stderr, code) in cases {
        let error = classify_failure(stderr.as_bytes());
        assert_eq!(error.code, code, "{stderr}");
        assert!(!error.message.contains("secret"), "{}", error.message);
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A `confluence` executable answering from canned fixture output. It logs
/// argv only and refuses to answer without the read-only environment.
struct Shim {
    root: PathBuf,
    config: ProjectConfiguration,
    responses: serde_json::Map<String, Value>,
}

impl Shim {
    fn new(base: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cockpit-confluence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let command = root.join("confluence");
        std::fs::write(
            &command,
            r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'argv.jsonl').open('a') as log:
    log.write(json.dumps(args) + '\n')
token = os.environ.get('CONFLUENCE_API_TOKEN')
with (root / 'env.jsonl').open('a') as log:
    log.write(json.dumps({
        'token': None if token is None else 'stored' if token == '__TOKEN__' else 'other',
        **{name: os.environ.get('CONFLUENCE_' + name) for name in
           ('DOMAIN', 'PROTOCOL', 'API_PATH', 'AUTH_TYPE', 'EMAIL')},
    }) + '\n')
if os.environ.get('CONFLUENCE_READ_ONLY') != 'true' or os.environ.get('CONFLUENCE_CLI_ANALYTICS') != 'false':
    sys.stderr.write('read-only environment missing')
    sys.exit(97)
responses = json.loads((root / 'responses.json').read_text())
answer = responses.get(json.dumps(args[2:], separators=(',', ':'), ensure_ascii=False))
if answer is None:
    sys.stderr.write(json.dumps({'error': 'unexpected call', 'code': 'UNKNOWN'}))
    sys.exit(98)
sys.stdout.write(answer.get('stdout', ''))
sys.stderr.write(answer.get('stderr', ''))
sys.exit(answer.get('code', 0))
"#
            .replace("__TOKEN__", TOKEN),
        )
        .unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = configuration(base, command.to_str().unwrap(), Some("cockpit-readonly"));
        Self {
            root,
            config,
            responses: Default::default(),
        }
    }

    fn answer(&mut self, argv: &[&str], stdout: &str) -> &mut Self {
        self.respond(argv, stdout, "", 0)
    }

    fn respond(&mut self, argv: &[&str], stdout: &str, stderr: &str, code: i32) -> &mut Self {
        self.responses.insert(
            serde_json::to_string(argv).unwrap(),
            json!({"stdout": stdout, "stderr": stderr, "code": code}),
        );
        std::fs::write(
            self.root.join("responses.json"),
            serde_json::to_vec(&self.responses).unwrap(),
        )
        .unwrap();
        self
    }

    fn provider(&self) -> ConfluenceSourceProvider {
        ConfluenceSourceProvider::configured(&self.config, "wiki").unwrap()
    }

    fn argv(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.root.join("argv.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Calls after the `--profile <login>` prefix, which every call must carry.
    fn calls(&self) -> Vec<String> {
        self.argv()
            .into_iter()
            .map(|argv| {
                assert_eq!(argv[..2], ["--profile", "cockpit-readonly"]);
                argv[2..].join(" ")
            })
            .collect()
    }

    fn request(&self, instance: &str, page_id: &str) -> SourceFetchRequest {
        let url = Url::parse(instance).unwrap();
        SourceFetchRequest {
            provider_id: "wiki".into(),
            artifact_url: cockpit_core::sources::confluence_page_url(instance, page_id),
            authority: SourceAuthority {
                provider_instance: instance.into(),
                origin_host: url.host_str().unwrap().into(),
                origin_port: url.port(),
                origin_base_path: url.path().trim_end_matches('/').into(),
                owner: String::new(),
                repository: String::new(),
            },
        }
    }
}

impl Drop for Shim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn configuration(base: &str, executable: &str, login: Option<&str>) -> ProjectConfiguration {
    ProjectConfiguration {
        version: 1,
        repository_roots: Vec::new(),
        worktree_root: "/w".into(),
        companion_root: "/c".into(),
        state_root: "/s".into(),
        cache_root: "/cache".into(),
        library_root: "/l".into(),
        notes_root: "/notes".into(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: vec![ProjectProvider {
            id: "wiki".into(),
            base_url: base.into(),
            executable: executable.into(),
            login: login.map(str::to_owned),
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
        origins: Default::default(),
    }
}

const PAGE_EXPAND: &str = "expand=ancestors,version,space,history.lastUpdated";

fn page_shim(base: &str, id: &str, dir: &str) -> Shim {
    let (info, content, labels, read, attachments) = match dir {
        "cloud" => (
            fixture!("cloud/page/info.json"),
            fixture!("cloud/page/content.json"),
            fixture!("cloud/page/labels.json"),
            fixture!("cloud/page/read.md"),
            fixture!("cloud/page/attachments.json"),
        ),
        _ => (
            fixture!("dc/info.json"),
            fixture!("dc/content.json"),
            fixture!("dc/labels.json"),
            fixture!("dc/read.md"),
            fixture!("dc/attachments.json"),
        ),
    };
    let mut shim = Shim::new(base);
    let content_path = format!("content/{id}");
    let label_path = format!("content/{id}/label");
    shim.answer(&["info", id, "--json"], info)
        .answer(
            &["api", &content_path, "-X", "GET", "-f", PAGE_EXPAND],
            content,
        )
        .answer(&["api", &label_path, "-X", "GET"], labels)
        .answer(&["read", id, "--format", "markdown"], read)
        .answer(&["attachments", id, "--json"], attachments);
    shim
}

fn field<'a>(
    asset: &'a cockpit_core::sources::SourceAsset,
    key: &str,
) -> Option<&'a FrontmatterValue> {
    asset
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
}

#[tokio::test]
async fn cloud_and_dc_confluence_pages_normalize_to_the_same_item_shape() {
    let cloud = page_shim(CLOUD, "123456789", "cloud");
    let dc = page_shim(DC, "524301", "dc");
    let cloud_asset = cloud
        .provider()
        .fetch(&cloud.request(CLOUD, "123456789"))
        .await
        .unwrap()
        .remove(0);
    let dc_asset = dc
        .provider()
        .fetch(&dc.request(DC, "524301"))
        .await
        .unwrap()
        .remove(0);

    for (shim, id) in [(&cloud, "123456789"), (&dc, "524301")] {
        let mut actual = shim.calls();
        actual.sort();
        let mut expected = vec![
            format!("api content/{id} -X GET -f {PAGE_EXPAND}"),
            format!("api content/{id}/label -X GET"),
            format!("read {id} --format markdown"),
            format!("attachments {id} --json"),
        ];
        expected.sort();
        assert_eq!(actual, expected);
    }
    for asset in [&cloud_asset, &dc_asset] {
        assert_eq!(asset.source.resource_type, "page");
        assert_eq!(asset.title, "Release Checklist");
        assert!(asset.complete);
        assert!(asset.diagnostics.is_empty());
        assert!(
            asset
                .body
                .starts_with("## Before the release\n\n- Freeze the branch")
        );
        assert!(
            !asset.body.ends_with("\n\n"),
            "one console.log newline is dropped"
        );
        assert_eq!(
            field(asset, "labels"),
            Some(&FrontmatterValue::Strings(vec![
                "release".into(),
                "checklist".into()
            ]))
        );
        assert_eq!(
            field(asset, "last_modified_by"),
            Some(&FrontmatterValue::String("<display-name>".into()))
        );
        assert_eq!(asset.attachments.len(), 1);
        let attachment = &asset.attachments[0];
        assert_eq!(attachment.title, "release-flow.png");
        assert_eq!(attachment.media_type.as_deref(), Some("image/png"));
        assert_eq!(attachment.size, Some(2048));
        assert_eq!(attachment.source_revision.as_deref(), Some("1"));
        assert_eq!(attachment.not_downloaded.as_deref(), Some("not downloaded"));
        assert!(attachment.path.is_none());
    }
    let keys = |asset: &cockpit_core::sources::SourceAsset| {
        asset
            .fields
            .iter()
            .map(|field| field.key.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&cloud_asset), keys(&dc_asset));
    assert_eq!(
        keys(&cloud_asset),
        [
            "space_key",
            "space_name",
            "page_id",
            "parent_id",
            "ancestors",
            "ancestor_ids",
            "version",
            "last_modified",
            "last_modified_by",
            "labels"
        ]
    );

    assert_eq!(cloud_asset.source.canonical_id, "123456789");
    assert_eq!(cloud_asset.source.provider_instance, CLOUD);
    assert_eq!(cloud_asset.source_revision.as_deref(), Some("7"));
    assert_eq!(
        cloud_asset.source_url.as_deref(),
        Some("https://nnexai.atlassian.net/wiki/spaces/SD/pages/123456789/Release+Checklist")
    );
    let container = cloud_asset.container.as_ref().unwrap();
    assert_eq!(
        (container.id.as_str(), container.label.as_str()),
        ("SD", "SD · Software Development")
    );
    assert_eq!(
        field(&cloud_asset, "ancestors"),
        Some(&FrontmatterValue::Strings(vec![
            "Software Development Home".into(),
            "Runbooks".into(),
            "Engineering".into()
        ]))
    );
    assert_eq!(
        field(&cloud_asset, "parent_id"),
        Some(&FrontmatterValue::String("98765".into()))
    );
    assert_eq!(
        field(&cloud_asset, "version"),
        Some(&FrontmatterValue::Number(7))
    );
    assert!(cloud_asset.attachments[0].source_url.is_some());

    assert_eq!(dc_asset.source.canonical_id, "524301");
    assert_eq!(dc_asset.source.provider_instance, DC);
    assert_eq!(dc_asset.source_revision.as_deref(), Some("12"));
    assert_eq!(
        dc_asset.source_url.as_deref(),
        Some("https://confluence.example.com/confluence/display/ENG/Release+Checklist")
    );
    assert_eq!(
        dc_asset.container.as_ref().unwrap().label,
        "ENG · Engineering"
    );
    assert_eq!(
        dc_asset.attachments[0].source_url, None,
        "a download link outside the configured context path is not provenance"
    );
    for text in [
        serde_json::to_string(&cloud_asset).unwrap(),
        serde_json::to_string(&dc_asset).unwrap(),
    ] {
        assert!(
            !text.contains("<account-id>")
                && !text.contains("<user-key>")
                && !text.contains("<username>")
        );
    }
}

#[tokio::test]
async fn dc_display_url_resolves_through_find_after_double_dash_then_info() {
    let mut shim = page_shim(DC, "524301", "dc");
    shim.answer(
        &[
            "find",
            "--space",
            "ENG",
            "--json",
            "--",
            "Release Checklist",
        ],
        fixture!("dc/find-release-checklist.json"),
    );
    let resolution = shim
        .provider()
        .resolve_input("https://confluence.example.com/confluence/display/ENG/Release+Checklist")
        .await
        .unwrap();
    let ProviderResolution::ConfluencePage(page) = resolution else {
        panic!("expected a page");
    };
    assert_eq!(page.page_id, "524301");
    assert_eq!(page.space_key, "ENG");
    assert_eq!(page.title, "Release Checklist");
    assert_eq!(page.version, Some(12));
    assert_eq!(
        page.canonical_url,
        "https://confluence.example.com/confluence/pages/viewpage.action?pageId=524301"
    );
    assert_eq!(
        shim.calls(),
        [
            "find --space ENG --json -- Release Checklist",
            "info 524301 --json"
        ]
    );

    // A title that starts like an option stays a positional after `--`.
    let mut draft = Shim::new(DC);
    let mut info: Value = serde_json::from_str(fixture!("dc/info.json")).unwrap();
    info["id"] = json!("524399");
    info["title"] = json!("-draft notes");
    draft
        .answer(
            &["find", "--space", "ENG", "--json", "--", "-draft notes"],
            &json!({"id": "524399", "title": "-draft notes", "space": {"key": "ENG"}, "url": ""})
                .to_string(),
        )
        .answer(&["info", "524399", "--json"], &info.to_string());
    let resolution = draft
        .provider()
        .resolve_input("https://confluence.example.com/confluence/display/ENG/-draft+notes")
        .await
        .unwrap();
    assert!(
        matches!(resolution, ProviderResolution::ConfluencePage(page) if page.page_id == "524399")
    );
    assert_eq!(draft.calls()[0], "find --space ENG --json -- -draft notes");
}

#[tokio::test]
async fn display_url_whose_find_or_info_disagrees_is_not_found() {
    let url = "https://confluence.example.com/confluence/display/ENG/Release+Checklist";
    // Another space's page with the same title.
    let mut other_space = page_shim(DC, "524301", "dc");
    let mut info: Value = serde_json::from_str(fixture!("dc/info.json")).unwrap();
    info["spaceKey"] = json!("OPS");
    other_space
        .answer(
            &[
                "find",
                "--space",
                "ENG",
                "--json",
                "--",
                "Release Checklist",
            ],
            fixture!("dc/find-release-checklist.json"),
        )
        .answer(&["info", "524301", "--json"], &info.to_string());
    assert_eq!(
        other_space
            .provider()
            .resolve_input(url)
            .await
            .unwrap_err()
            .code,
        "source_not_found"
    );
    // A different title (CQL title matching is not exact).
    let mut other_title = page_shim(DC, "524301", "dc");
    info["spaceKey"] = json!("ENG");
    info["title"] = json!("release checklist");
    other_title
        .answer(
            &[
                "find",
                "--space",
                "ENG",
                "--json",
                "--",
                "Release Checklist",
            ],
            fixture!("dc/find-release-checklist.json"),
        )
        .answer(&["info", "524301", "--json"], &info.to_string());
    assert_eq!(
        other_title
            .provider()
            .resolve_input(url)
            .await
            .unwrap_err()
            .code,
        "source_not_found"
    );
    // `find` answering with a space object (observed live on Cloud) has no page id.
    let mut space_hit = Shim::new(DC);
    space_hit.answer(
        &[
            "find",
            "--space",
            "ENG",
            "--json",
            "--",
            "Release Checklist",
        ],
        fixture!("cloud/find-software-development.stdout.json"),
    );
    assert_eq!(
        space_hit
            .provider()
            .resolve_input(url)
            .await
            .unwrap_err()
            .code,
        "source_not_found"
    );
    assert_eq!(space_hit.calls().len(), 1, "no Info without a page id");
    // `find` reporting no page.
    let mut missing = Shim::new(DC);
    missing.respond(
        &[
            "find",
            "--space",
            "ENG",
            "--json",
            "--",
            "Release Checklist",
        ],
        "",
        fixture!("cloud/find-release-checklist.stderr.txt"),
        1,
    );
    assert_eq!(
        missing
            .provider()
            .resolve_input(url)
            .await
            .unwrap_err()
            .code,
        "source_not_found"
    );
}

#[tokio::test]
async fn page_ids_and_space_keys_resolve_without_spaces_fanout() {
    let shim = page_shim(CLOUD, "123456789", "cloud");
    let provider = shim.provider();
    let ProviderResolution::ConfluencePage(page) =
        provider.resolve_input("123456789").await.unwrap()
    else {
        panic!("expected a page");
    };
    assert_eq!(
        page.canonical_url,
        "https://nnexai.atlassian.net/wiki/pages/viewpage.action?pageId=123456789"
    );
    assert_eq!(
        page.source_url,
        "https://nnexai.atlassian.net/wiki/spaces/SD/pages/123456789/Release+Checklist"
    );
    assert_eq!(
        provider
            .resolve_input("https://nnexai.atlassian.net/wiki/spaces/SD")
            .await
            .unwrap(),
        ProviderResolution::ConfluenceSpace {
            space_key: "SD".into()
        }
    );
    assert_eq!(shim.calls(), ["info 123456789 --json"]);
}

#[tokio::test]
async fn info_from_another_instance_or_page_is_an_identity_mismatch() {
    let cases = [
        ("url", json!("https://evil.atlassian.net/wiki/spaces/SD/pages/123456789/Release+Checklist")),
        ("url", json!("http://nnexai.atlassian.net/wiki/spaces/SD/pages/123456789")),
        ("url", json!("https://nnexai.atlassian.net:8443/wiki/spaces/SD/pages/123456789")),
        ("url", json!("https://nnexai.atlassian.net/spaces/SD/pages/123456789")),
        ("url", json!("https://nnexai.atlassian.net/wikiX/spaces/SD/pages/123456789")),
        ("url", Value::Null),
        ("id", json!("123456780")),
    ];
    for (key, value) in cases {
        let mut shim = page_shim(CLOUD, "123456789", "cloud");
        let mut info: Value = serde_json::from_str(fixture!("cloud/page/info.json")).unwrap();
        info[key] = value.clone();
        shim.answer(&["info", "123456789", "--json"], &info.to_string());
        assert_eq!(
            shim.provider().resolve_input("123456789").await.unwrap_err().code,
            "source_identity_mismatch",
            "{key}={value}"
        );
        assert_eq!(shim.calls(), ["info 123456789 --json"]);
    }
}

#[tokio::test]
async fn fetched_content_rejects_a_different_site_origin() {
    let mut shim = page_shim(CLOUD, "123456789", "cloud");
    let mut content: Value = serde_json::from_str(fixture!("cloud/page/content.json")).unwrap();
    content["_links"]["base"] = json!("https://evil.atlassian.net/wiki");
    shim.answer(
        &["api", "content/123456789", "-X", "GET", "-f", PAGE_EXPAND],
        &content.to_string(),
    );
    assert_eq!(
        shim.provider()
            .fetch(&shim.request(CLOUD, "123456789"))
            .await
            .unwrap_err()
            .code,
        "source_identity_mismatch"
    );
    assert_eq!(
        shim.calls(),
        [format!("api content/123456789 -X GET -f {PAGE_EXPAND}")]
    );

    let mut wrong_page = page_shim(CLOUD, "123456789", "cloud");
    let mut content: Value = serde_json::from_str(fixture!("cloud/page/content.json")).unwrap();
    content["id"] = json!("1");
    wrong_page.answer(
        &["api", "content/123456789", "-X", "GET", "-f", PAGE_EXPAND],
        &content.to_string(),
    );
    assert_eq!(
        wrong_page.provider()
            .fetch(&wrong_page.request(CLOUD, "123456789"))
            .await
            .unwrap_err()
            .code,
        "source_identity_mismatch"
    );
}

#[tokio::test]
async fn fetch_refuses_requests_outside_the_configured_instance_before_spawning() {
    let shim = page_shim(CLOUD, "123456789", "cloud");
    let provider = shim.provider();
    let mut foreign = shim.request(CLOUD, "123456789");
    foreign.authority.origin_host = "evil.atlassian.net".into();
    let mut other_base = shim.request("https://nnexai.atlassian.net", "123456789");
    other_base.authority.origin_base_path.clear();
    let mut repository = shim.request(CLOUD, "123456789");
    repository.authority.owner = "acme".into();
    repository.authority.repository = "repo".into();
    let mut display = shim.request(CLOUD, "123456789");
    display.artifact_url = "https://nnexai.atlassian.net/wiki/display/SD/Release+Checklist".into();
    let mut bare = shim.request(CLOUD, "123456789");
    bare.artifact_url = "123456789".into();
    let mut elsewhere = shim.request(CLOUD, "123456789");
    elsewhere.artifact_url = "https://evil.test/wiki/pages/viewpage.action?pageId=1".into();
    for (request, code) in [
        (foreign, "source_identity_mismatch"),
        (other_base, "source_identity_mismatch"),
        (repository, "source_identity_mismatch"),
        (display, "library_input_unrecognized"),
        (bare, "library_input_unrecognized"),
        (elsewhere, "source_identity_mismatch"),
    ] {
        assert_eq!(
            provider.fetch(&request).await.unwrap_err().code,
            code,
            "{}",
            request.artifact_url
        );
    }
    assert!(shim.argv().is_empty());
}

#[tokio::test]
async fn auth_failure_missing_cli_and_missing_login_map_to_stable_codes() {
    let mut shim = Shim::new(CLOUD);
    shim.respond(
        &["info", "1", "--json"],
        "",
        fixture!("cloud/page/info-auth-failed.stderr.json"),
        1,
    );
    assert_eq!(
        shim.provider().resolve_input("1").await.unwrap_err().code,
        "source_auth_failed"
    );

    let mut not_found = Shim::new(CLOUD);
    not_found.respond(
        &["info", "1", "--json"],
        "",
        fixture!("cloud/info-not-found.stderr.txt"),
        1,
    );
    assert_eq!(
        not_found
            .provider()
            .resolve_input("1")
            .await
            .unwrap_err()
            .code,
        "source_not_found"
    );

    let missing = configuration(CLOUD, "/nonexistent/cockpit-test/bin/confluence", Some("p"));
    let provider = ConfluenceSourceProvider::configured(&missing, "wiki").unwrap();
    assert_eq!(
        provider.resolve_input("1").await.unwrap_err().code,
        "source_cli_unavailable"
    );

    let mut no_login = Shim::new(CLOUD);
    no_login.config.providers[0].login = None;
    let provider = ConfluenceSourceProvider::configured(&no_login.config, "wiki").unwrap();
    assert_eq!(
        provider.resolve_input("1").await.unwrap_err().code,
        "source_login_unconfigured"
    );
    assert!(no_login.argv().is_empty());

    for invalid in [
        "nnexai.atlassian.net/wiki",
        "https://user@nnexai.atlassian.net/wiki",
        "https://nnexai.atlassian.net/wiki?x=1",
    ] {
        let configuration = configuration(invalid, "confluence", Some("p"));
        assert_eq!(
            ConfluenceSourceProvider::configured(&configuration, "wiki")
                .unwrap_err()
                .code,
            "source_provider_invalid"
        );
    }
}

#[tokio::test]
async fn selected_provider_resolution_and_fetch_pass_the_source_service_page_proof() {
    let shim = page_shim(DC, "524301", "dc");
    let providers = crate::configured_providers(&shim.config, ProviderCredentials::disabled()).unwrap();
    let service = cockpit_core::sources::SourceService::new(&shim.config, providers).unwrap();
    assert!(
        cockpit_core::repositories::resolve_artifact(
            &shim.config,
            "https://confluence.example.com/confluence/pages/viewpage.action?pageId=524301"
        )
        .is_err_and(|error| error.code == "unsupported_artifact")
    );
    let ProviderResolution::ConfluencePage(page) =
        service.resolve_input("wiki", " 524301 ").await.unwrap()
    else {
        panic!("expected a page");
    };
    let request = shim.request(DC, &page.page_id);
    assert_eq!(request.artifact_url, page.canonical_url);
    let fetched = service.fetch_assets(request).await.unwrap();
    assert_eq!(fetched.assets[0].source.canonical_id, "524301");
}
#[test]
fn search_continuations_validate_endpoint_and_preserve_only_typed_cursors() {
    let cloud = Shim::new(CLOUD);
    let provider = cloud.provider();
    let base = json!({"_links":{"base":CLOUD}});
    let cloud_next = format!(
        "{CLOUD}/rest/api/content/search?cql=space%3D%22SD%22+and+type%3Dpage&limit=100&expand=version%2Cancestors%2Cspace&cursor=opaque"
    );
    let next = json!({"_links":{"base":CLOUD,"next":cloud_next}});
    assert_eq!(
        provider.search_continuation(&next, "SD", 100, "version,ancestors,space").unwrap(),
        Some(SearchPage::Cursor("opaque".into()))
    );
    // Shape returned by nnexai.atlassian.net: relative link, `next=true`, cursor plus start.
    let real = json!({"_links":{"base":CLOUD,"next":"/rest/api/content/search?next=true&cursor=_t_WyJcdDE5MzMzMTMiXQ%3D%3D_h_W10%3D&expand=version,ancestors,space&limit=25&start=25&cql=space%3D%22SD%22+and+type%3Dpage"}});
    assert_eq!(
        provider.search_continuation(&real, "SD", 25, "version,ancestors,space").unwrap(),
        Some(SearchPage::Cursor("_t_WyJcdDE5MzMzMTMiXQ==_h_W10=".into()))
    );
    for bad in [
        "https://evil.test/wiki/rest/api/content/search?cql=space%3D%22SD%22+and+type%3Dpage&limit=100&expand=version%2Cancestors%2Cspace&cursor=opaque",
        "https://nnexai.atlassian.net/wiki/rest/api/other?cql=space%3D%22SD%22+and+type%3Dpage&limit=100&expand=version%2Cancestors%2Cspace&cursor=opaque",
        "https://nnexai.atlassian.net/wiki/rest/api/content/search?cql=space%3D%22EVIL%22+and+type%3Dpage&limit=100&expand=version%2Cancestors%2Cspace&cursor=opaque",
    ] {
        let value = json!({"_links":{"base":CLOUD,"next":bad}});
        assert!(provider.search_continuation(&value, "SD", 100, "version,ancestors,space").is_err());
    }
    assert!(provider.search_continuation(&base, "SD", 100, "version,ancestors,space").unwrap().is_none());

    let dc = Shim::new(DC);
    let provider = dc.provider();
    let next = json!({"_links":{
        "base":DC,
        "next":"/rest/api/content/search?cql=space%3D%22SD%22+and+type%3Dpage&limit=100&expand=version%2Cancestors%2Cspace&start=2"
    }});
    assert_eq!(
        provider.search_continuation(&next, "SD", 100, "version,ancestors,space").unwrap(),
        Some(SearchPage::Start(2))
    );
}

async fn store(
    credentials: &ProviderCredentials,
    kind: ProviderAuthKind,
    username: Option<&str>,
) {
    credentials
        .set(ProviderCredentialSetRequest {
            provider_id: "wiki".into(),
            kind,
            username: username.map(str::to_owned),
            token: TOKEN.into(),
        })
        .await
        .unwrap();
}

impl Shim {
    /// The provider composed with a credential service over `vault`.
    fn credentialed(&self, vault: &Arc<MemoryVault>) -> (ConfluenceSourceProvider, Arc<ProviderCredentials>) {
        let credentials = Arc::new(ProviderCredentials::new(
            &self.config,
            vault.clone(),
            crate::credential_kinds,
        ));
        (self.provider().with_credentials(credentials.clone()), credentials)
    }

    /// One call that reaches the CLI, whatever it answers.
    async fn touch(provider: &ConfluenceSourceProvider) {
        let _ = provider.resolve_input("1").await;
    }

    /// What each call's environment looked like (names and classes only).
    fn envs(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("env.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn stored_credentials_name_the_configured_site_in_the_child_environment() {
    // (base_url, kind, username, domain, api path)
    let cases = [
        (CLOUD, ProviderAuthKind::Bearer, None, "nnexai.atlassian.net", "/wiki/rest/api", "https"),
        (
            "https://x.atlassian.net/wiki/",
            ProviderAuthKind::Basic,
            Some("me@example.test"),
            "x.atlassian.net",
            "/wiki/rest/api",
            "https",
        ),
        (DC, ProviderAuthKind::Bearer, None, "confluence.example.com/confluence", "/rest/api", "https"),
        (
            "https://H.example.com:8443/confluence",
            ProviderAuthKind::Basic,
            Some("svc"),
            "h.example.com:8443/confluence",
            "/rest/api",
            "https",
        ),
        ("http://127.0.0.1:8090", ProviderAuthKind::Bearer, None, "127.0.0.1:8090", "/rest/api", "http"),
    ];
    for (base, kind, username, domain, api_path, protocol) in cases {
        let shim = Shim::new(base);
        let (provider, credentials) = shim.credentialed(&Arc::new(MemoryVault::new()));
        store(&credentials, kind, username).await;
        Shim::touch(&provider).await;

        let auth_type = if kind == ProviderAuthKind::Bearer { "bearer" } else { "basic" };
        assert_eq!(
            shim.envs(),
            [json!({"token": "stored", "DOMAIN": domain, "PROTOCOL": protocol,
                    "API_PATH": api_path, "AUTH_TYPE": auth_type, "EMAIL": username})],
            "{base}"
        );
        // The profile prefix stays (ignored by the CLI in env mode) and no
        // argv entry carries the token.
        assert_eq!(shim.calls().len(), 1);
        let argv = std::fs::read_to_string(shim.root.join("argv.jsonl")).unwrap();
        assert!(!argv.contains(TOKEN), "{argv}");
    }
}

#[tokio::test]
async fn stored_credentials_replace_inherited_cookie_tls_and_identity_settings() {
    let url = Url::parse(CLOUD).unwrap();
    let inherited = [
        "CONFLUENCE_COOKIE",
        "CONFLUENCE_TLS_CLIENT_CERT",
        "CONFLUENCE_TLS_CLIENT_KEY",
        "CONFLUENCE_TLS_CA_CERT",
    ];
    let credential = |kind, username: Option<&'static str>| async move {
        let shim = Shim::new(CLOUD);
        let (_, credentials) = shim.credentialed(&Arc::new(MemoryVault::new()));
        store(&credentials, kind, username).await;
        credentials.for_cli("wiki").await.unwrap()
    };
    let envs = |credential: &ProviderCredential| {
        let mut command = tokio::process::Command::new("confluence");
        inject_credential(&mut command, &url, credential);
        command
            .as_std()
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_str().unwrap().to_owned(),
                    value.map(|value| value.to_str().unwrap().to_owned()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };

    let bearer = envs(&*credential(ProviderAuthKind::Bearer, None).await);
    for name in inherited.into_iter().chain(["CONFLUENCE_EMAIL", "CONFLUENCE_USERNAME"]) {
        assert_eq!(bearer.get(name), Some(&None), "{name} must be removed for bearer");
    }
    let basic = envs(&*credential(ProviderAuthKind::Basic, Some("me@example.test")).await);
    for name in inherited {
        assert_eq!(basic.get(name), Some(&None), "{name} must be removed for basic");
    }
    assert_eq!(basic["CONFLUENCE_EMAIL"].as_deref(), Some("me@example.test"));
    // `CONFLUENCE_USERNAME` only applies when no email is set, so it is left alone.
    assert!(!basic.contains_key("CONFLUENCE_USERNAME"));
    // Read-only stays in the caller's hands; injection never touches it.
    for map in [&bearer, &basic] {
        assert!(!map.contains_key("CONFLUENCE_READ_ONLY"));
        assert_eq!(map["CONFLUENCE_API_TOKEN"].as_deref(), Some(TOKEN));
    }
}

#[tokio::test]
async fn confluence_without_a_stored_token_keeps_the_profile_login_and_recovers_with_the_vault() {
    let ambient = |name: &str| std::env::var(name).map_or(Value::Null, Value::String);
    let vault = Arc::new(MemoryVault::new());
    let shim = Shim::new(CLOUD);
    let (provider, earlier) = shim.credentialed(&vault);

    // Not stored: the inherited environment passes through.
    Shim::touch(&provider).await;
    let untouched = json!({"token": ambient("CONFLUENCE_API_TOKEN"), "DOMAIN": ambient("CONFLUENCE_DOMAIN"),
        "PROTOCOL": ambient("CONFLUENCE_PROTOCOL"), "API_PATH": ambient("CONFLUENCE_API_PATH"),
        "AUTH_TYPE": ambient("CONFLUENCE_AUTH_TYPE"), "EMAIL": ambient("CONFLUENCE_EMAIL")});
    assert_eq!(shim.envs(), [untouched.clone()]);

    // Stored by an earlier process; this one starts cold while the vault is down.
    store(&earlier, ProviderAuthKind::Bearer, None).await;
    let (provider, _) = shim.credentialed(&vault);
    vault.set_failing(true);
    Shim::touch(&provider).await;
    assert_eq!(shim.envs()[1], untouched);

    vault.set_failing(false);
    Shim::touch(&provider).await;
    assert_eq!(shim.envs()[2]["token"], "stored");
    assert_eq!(shim.envs()[2]["DOMAIN"], "nnexai.atlassian.net");
    // Every call kept the profile prefix.
    assert_eq!(shim.calls().len(), 3);
}

#[tokio::test]
async fn confluence_allows_attachment_downloads_for_pages_only() {
    let provider = Shim::new(CLOUD).provider();
    provider.attachment_downloads("page").await.unwrap();
    for other in ["issue", "space", ""] {
        assert_eq!(
            provider.attachment_downloads(other).await.unwrap_err().code,
            "source_capability_unavailable"
        );
    }
}
