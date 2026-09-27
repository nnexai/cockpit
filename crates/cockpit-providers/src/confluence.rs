//! Read-only Confluence pages through the owner's configured `confluence`
//! CLI (pchuri/confluence-cli). The CLI owns the site login and credential,
//! selected by `ProjectProvider.login` as its profile name. Every process is
//! built by [`confluence_args`] from a typed [`ConfluenceCall`] and checked by
//! [`allowlisted_argv`] immediately before spawn; nothing else is ever run.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_core::InspectionError;
use cockpit_core::process::{run_bounded_command, run_bounded_staging_command, StagingBudget};
use cockpit_core::repositories::is_confluence_executable;
use cockpit_core::sources::{
    AttachmentRef, ConfluencePage, DownloadedAttachment, FrontmatterField, FrontmatterValue,
    ProviderResolution, SourceAsset, SourceAttachment, SourceContainer, SourceFetchRequest,
    SourceMetadata, SourceProvider, SourceRef, SpacePage, SpacePageListing, SpaceSummary,
    confluence_attachment_pattern, confluence_page_url,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::sources::SourceCapability;
use serde_json::Value;
use tokio::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;



/// Library provider bodies are bounded like every other source asset.
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ATTACHMENTS: usize = 256;
const MAX_TITLE_CHARS: usize = 255;
const MAX_FIELD_CHARS: usize = 256;
const NOT_DOWNLOADED: &str = "not downloaded";

pub(crate) fn executable(value: &str) -> bool {
    is_confluence_executable(value)
}

/// `expand` values allowed in `api content/<id>` and `api content/search`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentExpand {
    Ancestors,
    Version,
    Space,
    HistoryLastUpdated,
    MetadataLabels,
}

impl ContentExpand {
    const ALL: [Self; 5] = [
        Self::Ancestors,
        Self::Version,
        Self::Space,
        Self::HistoryLastUpdated,
        Self::MetadataLabels,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Ancestors => "ancestors",
            Self::Version => "version",
            Self::Space => "space",
            Self::HistoryLastUpdated => "history.lastUpdated",
            Self::MetadataLabels => "metadata.labels",
        }
    }
}

/// Continuation of a CQL search: Cloud cursors or Data Center offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchPage {
    Cursor(String),
    Start(u64),
}

/// The four `api` endpoint templates (D16); always `-X GET`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfluenceApi {
    Content {
        page_id: String,
        expand: Vec<ContentExpand>,
    },
    Labels {
        page_id: String,
    },
    Space {
        space_key: String,
    },
    Search {
        space_key: String,
        limit: u8,
        expand: Vec<ContentExpand>,
        page: Option<SearchPage>,
    },
}

/// Every Confluence CLI invocation Cockpit may make (examples §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfluenceCall {
    Spaces,
    Info {
        page_id: String,
    },
    Read {
        page_id: String,
    },
    Find {
        space_key: String,
        title: String,
    },
    Attachments {
        page_id: String,
    },
    DownloadAttachment {
        page_id: String,
        pattern: String,
        dest: PathBuf,
    },
    Api(ConfluenceApi),
}

fn contract(message: &str) -> InspectionError {
    InspectionError::new("source_provider_contract", message)
}

fn page_id_valid(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn space_key_valid(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'~' | b'_' | b'-'))
}

fn title_valid(value: &str) -> bool {
    let count = value.chars().count();
    (1..=MAX_TITLE_CHARS).contains(&count) && !value.chars().any(char::is_control)
}

fn cursor_valid(value: &str) -> bool {
    (1..=1024).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'~' | b'%' | b'+' | b'=' | b'/' | b'-')
        })
}

fn pattern_valid(value: &str) -> bool {
    title_valid(value) && !value.contains('*') && value.trim() == value
}
fn download_destination_valid(value: &Path) -> bool {
    value.is_absolute()
        && value.to_str().is_some_and(|text| text.len() <= 4096 && !text.chars().any(char::is_control))
        && value
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && value.components().count() > 1
}

fn download_capability_error() -> InspectionError {
    InspectionError::new(
        "source_capability_unavailable",
        "this confluence-cli version cannot download attachments safely",
    )
}

fn saved_file_name(saved_to: &str, destination: &Path) -> Result<String, InspectionError> {
    let path = Path::new(saved_to);
    let Some(name) = path.file_name().filter(|name| !name.is_empty()) else {
        return Err(download_capability_error());
    };
    if !path.is_absolute()
        || path.parent() != Some(destination)
        || path.components().count() != destination.components().count() + 1
        || !path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(download_capability_error());
    }
    name.to_str()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_owned)
        .ok_or_else(download_capability_error)
}

fn dest_valid(value: &Path) -> bool {
    value.is_absolute()
        && value
            .to_str()
            .is_some_and(|text| text.len() <= 4096 && !text.chars().any(char::is_control))
        && value
            .components()
            .skip(1)
            .all(|component| matches!(component, Component::Normal(_)))
        && value.components().count() > 1
}

fn profile_valid(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && !value.starts_with('-')
}

fn expand_value(expand: &[ContentExpand]) -> Result<String, InspectionError> {
    let unique = expand
        .iter()
        .enumerate()
        .all(|(index, value)| !expand[..index].contains(value));
    if expand.is_empty() || !unique {
        return Err(contract("Confluence expand must be a non-empty set"));
    }
    Ok(expand
        .iter()
        .map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(","))
}

fn search_cql(space_key: &str) -> String {
    format!("space=\"{space_key}\" and type=page")
}

/// Render one typed call as the argv after `--profile <login>`. Invalid
/// values are refused here, before any process exists.
pub fn confluence_args(call: &ConfluenceCall) -> Result<Vec<OsString>, InspectionError> {
    let page = |page_id: &str| {
        if page_id_valid(page_id) {
            Ok(page_id.to_owned())
        } else {
            Err(contract("Confluence page id must be 1–20 digits"))
        }
    };
    let key = |space_key: &str| {
        if space_key_valid(space_key) {
            Ok(space_key.to_owned())
        } else {
            Err(contract("Confluence space key is malformed"))
        }
    };
    let strings: Vec<String> = match call {
        ConfluenceCall::Spaces => vec!["spaces".into(), "--all".into(), "--json".into()],
        ConfluenceCall::Info { page_id } => vec!["info".into(), page(page_id)?, "--json".into()],
        ConfluenceCall::Read { page_id } => vec![
            "read".into(),
            page(page_id)?,
            "--format".into(),
            "markdown".into(),
        ],
        ConfluenceCall::Find { space_key, title } => {
            if !title_valid(title) {
                return Err(contract("Confluence title must be 1–255 characters"));
            }
            vec![
                "find".into(),
                "--space".into(),
                key(space_key)?,
                "--json".into(),
                "--".into(),
                title.clone(),
            ]
        }
        ConfluenceCall::Attachments { page_id } => {
            vec!["attachments".into(), page(page_id)?, "--json".into()]
        }
        ConfluenceCall::DownloadAttachment {
            page_id,
            pattern,
            dest,
        } => {
            if !pattern_valid(pattern) {
                return Err(contract("Confluence attachment pattern is unsafe"));
            }
            if !dest_valid(dest) {
                return Err(contract(
                    "Confluence download destination must be an absolute private directory",
                ));
            }
            vec![
                "attachments".into(),
                page(page_id)?,
                "--download".into(),
                "--dest".into(),
                dest.to_string_lossy().into_owned(),
                format!("--pattern={pattern}"),
                "--json".into(),
            ]
        }
        ConfluenceCall::Api(api) => {
            let (endpoint, fields) = match api {
                ConfluenceApi::Content { page_id, expand } => (
                    format!("content/{}", page(page_id)?),
                    vec![format!("expand={}", expand_value(expand)?)],
                ),
                ConfluenceApi::Labels { page_id } => {
                    (format!("content/{}/label", page(page_id)?), Vec::new())
                }
                ConfluenceApi::Space { space_key } => (
                    format!("space/{}", key(space_key)?),
                    vec!["expand=homepage".into()],
                ),
                ConfluenceApi::Search {
                    space_key,
                    limit,
                    expand,
                    page: continuation,
                } => {
                    if !(1..=100).contains(limit) {
                        return Err(contract("Confluence search limit must be 1–100"));
                    }
                    let mut fields = vec![
                        format!("cql={}", search_cql(&key(space_key)?)),
                        format!("limit={limit}"),
                        format!("expand={}", expand_value(expand)?),
                    ];
                    match continuation {
                        Some(SearchPage::Cursor(cursor)) if cursor_valid(cursor) => {
                            fields.push(format!("cursor={cursor}"))
                        }
                        Some(SearchPage::Cursor(_)) => {
                            return Err(contract("Confluence search cursor is malformed"));
                        }
                        Some(SearchPage::Start(start)) => fields.push(format!("start={start}")),
                        None => {}
                    }
                    ("content/search".into(), fields)
                }
            };
            let mut argv = vec![endpoint, "-X".into(), "GET".into()];
            for field in fields {
                argv.push("-f".into());
                argv.push(field);
            }
            let mut rendered = vec!["api".to_owned()];
            rendered.extend(argv);
            rendered
        }
    };
    Ok(strings.into_iter().map(OsString::from).collect())
}

fn refused(message: &str) -> InspectionError {
    InspectionError::new(
        "source_provider_contract",
        format!("refused Confluence CLI argv: {message}"),
    )
}

fn expand_allowed(value: &str) -> bool {
    let parts: Vec<&str> = value.split(',').collect();
    !parts.is_empty()
        && parts.iter().enumerate().all(|(index, part)| {
            ContentExpand::ALL
                .iter()
                .any(|expand| expand.as_str() == *part)
                && !parts[..index].contains(part)
        })
}

fn api_allowed(args: &[&str]) -> bool {
    let [endpoint, method_flag, method, rest @ ..] = args else {
        return false;
    };
    if *method_flag != "-X" || *method != "GET" || rest.len() % 2 != 0 {
        return false;
    }
    let mut fields = Vec::new();
    for pair in rest.chunks(2) {
        if pair[0] != "-f" {
            return false;
        }
        let Some((name, value)) = pair[1].split_once('=') else {
            return false;
        };
        fields.push((name, value));
    }
    let segments: Vec<&str> = endpoint.split('/').collect();
    match segments.as_slice() {
        ["content", "search"] => {
            let (fixed, continuation) = match fields.as_slice() {
                [a, b, c] => ([*a, *b, *c], None),
                [a, b, c, d] => ([*a, *b, *c], Some(*d)),
                _ => return false,
            };
            let [("cql", cql), ("limit", limit), ("expand", expand)] = fixed else {
                return false;
            };
            let template = cql
                .strip_prefix("space=\"")
                .and_then(|rest| rest.strip_suffix("\" and type=page"))
                .is_some_and(space_key_valid);
            template
                && limit
                    .parse::<u8>()
                    .is_ok_and(|parsed| (1..=100).contains(&parsed) && parsed.to_string() == *limit)
                && expand_allowed(expand)
                && continuation.is_none_or(|(name, value)| match name {
                    "cursor" => cursor_valid(value),
                    "start" => value
                        .parse::<u64>()
                        .is_ok_and(|start| start.to_string() == value),
                    _ => false,
                })
        }
        ["content", id] => {
            page_id_valid(id)
                && matches!(fields.as_slice(), [("expand", expand)] if expand_allowed(expand))
        }
        ["content", id, "label"] => page_id_valid(id) && fields.is_empty(),
        ["space", key] => space_key_valid(key) && fields.as_slice() == [("expand", "homepage")],
        _ => false,
    }
}

/// The exhaustive allowlist, applied to the complete argv right before spawn.
/// Anything the typed builder cannot produce is refused.
pub fn allowlisted_argv(argv: &[OsString]) -> Result<(), InspectionError> {
    let mut strings = Vec::with_capacity(argv.len());
    for arg in argv {
        let Some(text) = arg.to_str() else {
            return Err(refused("non-UTF-8 argument"));
        };
        if text.contains('\0') {
            return Err(refused("NUL in argument"));
        }
        strings.push(text);
    }
    let ["--profile", profile, rest @ ..] = strings.as_slice() else {
        return Err(refused("every call names its profile first"));
    };
    if !profile_valid(profile) {
        return Err(refused("profile name is malformed"));
    }
    let allowed = match rest {
        ["spaces", "--all", "--json"] => true,
        ["info", id, "--json"] | ["attachments", id, "--json"] => page_id_valid(id),
        ["read", id, "--format", "markdown"] => page_id_valid(id),
        ["find", "--space", key, "--json", "--", title] => {
            space_key_valid(key) && title_valid(title)
        }
        [
            "attachments",
            id,
            "--download",
            "--dest",
            dest,
            pattern,
            "--json",
        ] => {
            page_id_valid(id)
                && dest_valid(Path::new(dest))
                && pattern
                    .strip_prefix("--pattern=")
                    .is_some_and(pattern_valid)
        }
        ["api", api @ ..] => api_allowed(api),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(refused("not an allowlisted read call"))
    }
}

/// A recognized Confluence input, before any CLI call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfluenceInput {
    Page {
        page_id: String,
    },
    /// `/display/<KEY>/<title>`: resolved with `Find`, then proved with `Info`.
    Display {
        space_key: String,
        title: String,
    },
    Space {
        space_key: String,
    },
}

fn unrecognized() -> InspectionError {
    InspectionError::new(
        "library_input_unrecognized",
        "not a Confluence page or space URL, page id or space key",
    )
}

/// `+` is a space and `%XX` a byte, like the CLI's `extractPageId`.
fn decode_title(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let hex = segment.get(index + 1..index + 3)?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8(decoded).ok()
}

fn within_instance(base: &Url, url: &Url) -> bool {
    let base_path = base.path().trim_end_matches('/');
    url.scheme() == base.scheme()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some()
        && url
            .host_str()
            .zip(base.host_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        && url.port_or_known_default() == base.port_or_known_default()
        && (base_path.is_empty()
            || url.path() == base_path
            || url.path().starts_with(&format!("{base_path}/")))
}

/// Recognize a Confluence input for one configured instance. Pure: no CLI.
pub fn parse_confluence_input(base: &Url, input: &str) -> Result<ConfluenceInput, InspectionError> {
    let input = input.trim();
    if input.is_empty() || input.len() > 8192 || input.chars().any(char::is_control) {
        return Err(unrecognized());
    }
    if input.bytes().all(|byte| byte.is_ascii_digit()) {
        return if page_id_valid(input) {
            Ok(ConfluenceInput::Page {
                page_id: input.into(),
            })
        } else {
            Err(unrecognized())
        };
    }
    if space_key_valid(input) {
        return Ok(ConfluenceInput::Space {
            space_key: input.into(),
        });
    }
    let url = Url::parse(input).map_err(|_| unrecognized())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(unrecognized());
    }
    if !within_instance(base, &url) {
        return Err(InspectionError::new(
            "source_identity_mismatch",
            "URL does not belong to the configured Confluence instance",
        ));
    }
    let relative = url
        .path()
        .strip_prefix(base.path().trim_end_matches('/'))
        .unwrap_or_default()
        .trim_start_matches('/');
    let segments: Vec<&str> = relative.split('/').collect();
    match segments.as_slice() {
        ["spaces", key, "pages", id, ..] if space_key_valid(key) && page_id_valid(id) => {
            Ok(ConfluenceInput::Page {
                page_id: (*id).into(),
            })
        }
        ["pages", "viewpage.action"] => {
            let ids: Vec<_> = url
                .query_pairs()
                .filter(|(name, _)| name == "pageId")
                .map(|(_, value)| value.into_owned())
                .collect();
            match ids.as_slice() {
                [id] if page_id_valid(id) => Ok(ConfluenceInput::Page {
                    page_id: id.clone(),
                }),
                _ => Err(unrecognized()),
            }
        }
        ["spaces", key]
        | ["spaces", key, "overview"]
        | ["spaces", key, ""]
        | ["display", key]
        | ["display", key, ""]
            if space_key_valid(key) =>
        {
            Ok(ConfluenceInput::Space {
                space_key: (*key).into(),
            })
        }
        ["display", key, rest @ ..] if space_key_valid(key) => {
            // Child pages may appear as /display/KEY/Parent/Child; the page
            // is the last segment.
            let title = rest
                .iter()
                .rev()
                .find(|segment| !segment.is_empty())
                .and_then(|segment| decode_title(segment))
                .filter(|title| title_valid(title))
                .ok_or_else(unrecognized)?;
            Ok(ConfluenceInput::Display {
                space_key: (*key).into(),
                title,
            })
        }
        _ => Err(unrecognized()),
    }
}

/// Map a failed CLI run to a stable error without echoing CLI output.
fn classify_failure(stderr: &[u8]) -> InspectionError {
    let text = String::from_utf8_lossy(stderr);
    let parsed: Option<Value> = serde_json::from_str(text.trim()).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let status = parsed.as_ref().and_then(|value| {
        ["status", "statusCode", "code"]
            .iter()
            .find_map(|name| value.get(*name).and_then(Value::as_u64))
    });
    let message = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| text.to_string());
    let auth = || {
        InspectionError::new(
            "source_auth_failed",
            "Confluence sign-in failed for the configured profile",
        )
    };
    let not_found = || InspectionError::new("source_not_found", "Confluence page was not found");
    let profile_missing = (message.contains("Profile \"") && message.contains("not found"))
        || message.contains("No configuration found");
    if code == "AUTH_FAILED" || matches!(status, Some(401 | 403)) || profile_missing {
        return auth();
    }
    if code == "NOT_FOUND" || status == Some(404) || message.starts_with("Page not found") {
        return not_found();
    }
    if parsed.is_none() {
        if ["status code 401", "status code 403", "401 Unauthorized"]
            .iter()
            .any(|needle| text.contains(needle))
        {
            return auth();
        }
        if text.contains("status code 404") {
            return not_found();
        }
    }
    if code == "NETWORK" {
        return InspectionError::new(
            "source_provider_failed",
            "Confluence could not be reached from the configured profile",
        );
    }
    InspectionError::new("source_provider_failed", "Confluence CLI request failed")
}

fn bounded_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.chars().count() <= MAX_FIELD_CHARS
                && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => number.as_u64().map(|number| number.to_string()),
        _ => None,
    }
}

/// `info --json`, reduced to the identity facts Cockpit relies on.
#[derive(Debug)]
struct PageInfo {
    page_id: String,
    title: String,
    space_key: String,
    version: Option<u64>,
    url: String,
}

/// Read-only Confluence pages from one configured instance.
#[derive(Debug)]
pub struct ConfluenceSourceProvider {
    provider_id: String,
    executable: String,
    login: Option<String>,
    base_url: Url,
    /// Normalized like `sources::site_authority`.
    instance: String,
    host: String,
    port: Option<u16>,
    base_path: String,
    timeout: Duration,
}

impl ConfluenceSourceProvider {
    pub fn configured(
        configuration: &ProjectConfiguration,
        provider_id: &str,
    ) -> Result<Self, InspectionError> {
        let provider = configuration
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "Confluence provider is not configured",
                )
            })?;
        let invalid = || {
            InspectionError::new(
                "source_provider_invalid",
                "Confluence base URL must be credential-free HTTP(S) without query or fragment",
            )
        };
        let base_url = Url::parse(&provider.base_url).map_err(|_| invalid())?;
        if !matches!(base_url.scheme(), "http" | "https")
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(invalid());
        }
        let host = base_url
            .host_str()
            .ok_or_else(invalid)?
            .to_ascii_lowercase();
        let port = base_url.port();
        let base_path = base_url.path().trim_end_matches('/').to_owned();
        let instance = format!(
            "{}://{host}{}{base_path}",
            base_url.scheme(),
            port.map(|port| format!(":{port}")).unwrap_or_default()
        );
        Ok(Self {
            provider_id: provider.id.clone(),
            executable: provider.executable.clone(),
            login: provider.login.clone(),
            base_url,
            instance,
            host,
            port,
            base_path,
            timeout: Duration::from_millis(configuration.limits.operation_timeout_ms.into()),
        })
    }

    fn url_in_instance(&self, input: &str) -> bool {
        Url::parse(input).is_ok_and(|url| within_instance(&self.base_url, &url))
    }

    /// Run one allowlisted call; returns stdout.
    async fn run(
        &self,
        call: &ConfluenceCall,
        stdout_limit: usize,
    ) -> Result<Vec<u8>, InspectionError> {
        self.run_with_staging(call, stdout_limit, None).await
    }

    async fn run_with_staging(
        &self,
        call: &ConfluenceCall,
        stdout_limit: usize,
        staging: Option<(&Dir, StagingBudget)>,
    ) -> Result<Vec<u8>, InspectionError> {
        let login = self.login.as_deref().ok_or_else(|| {
            InspectionError::new(
                "source_login_unconfigured",
                "Confluence sources require the confluence-cli profile name as the provider login",
            )
        })?;
        let mut argv: Vec<OsString> = vec!["--profile".into(), login.into()];
        argv.extend(confluence_args(call)?);
        allowlisted_argv(&argv)?;
        let mut command = Command::new(&self.executable);
        command
            .args(&argv)
            .env("CONFLUENCE_READ_ONLY", "true")
            .env("CONFLUENCE_CLI_ANALYTICS", "false");
        let output = match staging {
            Some((dir, budget)) => run_bounded_staging_command(
                command, stdout_limit, MAX_STDERR_BYTES, self.timeout, "Confluence CLI", dir, budget,
            ).await,
            None => run_bounded_command(
                command, stdout_limit, MAX_STDERR_BYTES, self.timeout, "Confluence CLI",
            ).await,
        }
        .map_err(|error| match error.code.as_str() {
            "execution_timeout" => InspectionError::new(
                "source_provider_timeout",
                "Confluence CLI request exceeded the configured deadline",
            ),
            "bounded_output" => InspectionError::new(
                "source_truncated",
                "Confluence CLI response exceeded Cockpit's explicit process limit",
            ),
            "execution_failed" => InspectionError::new(
                "source_cli_unavailable",
                "Confluence CLI (confluence-cli) is not installed or could not be started",
            ),
            "source_attachment_size" => InspectionError::new(
                "source_attachment_size",
                "Attachment download exceeded its staging budget or created an unsafe entry",
            ),
            _ => InspectionError::new("source_provider_failed", "Confluence CLI request failed"),
        })?;
        if !output.status.success() {
            return Err(classify_failure(&output.stderr));
        }
        Ok(output.stdout)
    }

    async fn json(&self, call: &ConfluenceCall) -> Result<Value, InspectionError> {
        let stdout = self.run(call, MAX_JSON_BYTES).await?;
        serde_json::from_slice(&stdout)
            .map_err(|_| contract("Confluence CLI did not return the documented JSON"))
    }

    async fn download_attachment_file(
        &self,
        page_id: &str,
        attachment: &AttachmentRef,
        siblings: &[AttachmentRef],
        dest: &Dir,
        dest_path: &Path,
        budget: StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        if !page_id_valid(page_id)
            || attachment.id.is_empty()
            || attachment.id.len() > MAX_FIELD_CHARS
            || attachment.id.chars().any(char::is_control)
            || !title_valid(&attachment.title)
            || !download_destination_valid(dest_path)
            || siblings.iter().any(|sibling| sibling.id == attachment.id)
        {
            return Err(contract("Confluence attachment request is malformed"));
        }
        if dest
            .entries()
            .map_err(|_| download_capability_error())?
            .next()
            .is_some()
        {
            return Err(download_capability_error());
        }
        let expected_dir = dest.dir_metadata().map_err(|_| download_capability_error())?;
        let actual_dir = std::fs::metadata(dest_path).map_err(|_| download_capability_error())?;
        #[cfg(unix)]
        if (MetadataExt::dev(&expected_dir), MetadataExt::ino(&expected_dir))
            != (MetadataExt::dev(&actual_dir), MetadataExt::ino(&actual_dir))
        {
            return Err(download_capability_error());
        }
        #[cfg(not(unix))]
        return Err(download_capability_error());
        let pattern = confluence_attachment_pattern(&attachment.title);
        let stdout = self
            .run_with_staging(
                &ConfluenceCall::DownloadAttachment {
                    page_id: page_id.to_owned(),
                    pattern,
                    dest: dest_path.to_owned(),
                },
                MAX_JSON_BYTES,
                Some((dest, budget)),
            )
            .await?;
        let result: Value =
            serde_json::from_slice(&stdout).map_err(|_| download_capability_error())?;
        let destination = result
            .get("destination")
            .and_then(Value::as_str)
            .ok_or_else(download_capability_error)?;
        if Path::new(destination) != dest_path {
            return Err(download_capability_error());
        }
        let entries = result
            .get("attachments")
            .and_then(Value::as_array)
            .ok_or_else(download_capability_error)?;
        let mut selected: Option<String> = None;
        let mut reported_names = Vec::with_capacity(entries.len());
        for entry in entries {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(download_capability_error)?;
            let saved_to = entry
                .get("savedTo")
                .and_then(Value::as_str)
                .ok_or_else(download_capability_error)?;
            let name = saved_file_name(saved_to, dest_path)?;
            let title = entry
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(download_capability_error)?;
            if id == attachment.id {
                if selected.is_some() || title != attachment.title {
                    return Err(download_capability_error());
                }
                selected = Some(name.clone());
            }
            reported_names.push(name);
        }
        let file_name = selected.ok_or_else(download_capability_error)?;
        if reported_names
            .iter()
            .enumerate()
            .any(|(index, name)| reported_names[..index].contains(name))
        {
            return Err(download_capability_error());
        }
        for name in reported_names.iter().filter(|name| *name != &file_name) {
            match dest.remove_file(name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(download_capability_error()),
            }
        }
        let metadata = dest
            .symlink_metadata(&file_name)
            .map_err(|_| download_capability_error())?;
        if !metadata.file_type().is_file() {
            return Err(download_capability_error());
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .follow(FollowSymlinks::No)
            .nonblock(true);
        let file = dest
            .open_with(&file_name, &options)
            .map_err(|_| download_capability_error())?;
        let opened = file.metadata().map_err(|_| download_capability_error())?;
        if !opened.is_file() || opened.len() != metadata.len()
            || {
                #[cfg(unix)]
                {
                    MetadataExt::nlink(&opened) != 1
                }
                #[cfg(windows)]
                {
                    opened.number_of_links() != 1
                }
                #[cfg(not(any(unix, windows)))]
                {
                    false
                }
            }
        {
            return Err(download_capability_error());
        }
        drop(file);
        Ok(DownloadedAttachment {
            attachment_id: attachment.id.clone(),
            file_name,
        })
    }

    fn search_continuation(
        &self,
        value: &Value,
        space_key: &str,
        limit: u8,
        expand: &str,
    ) -> Result<Option<SearchPage>, InspectionError> {
        let Some(next) = value.pointer("/_links/next").and_then(Value::as_str) else {
            return Ok(None);
        };
        if next.is_empty() || next.len() > 8192 {
            return Err(contract("Confluence search continuation is malformed"));
        }
        let base = value
            .pointer("/_links/base")
            .and_then(Value::as_str)
            .and_then(|base| Url::parse(base).ok())
            .filter(|base| within_instance(&self.base_url, base))
            .ok_or_else(|| contract("Confluence search continuation has an invalid base"))?;
        let url = base
            .join(next)
            .map_err(|_| contract("Confluence search continuation is malformed"))?;
        let expected_path = format!("{}/rest/api/content/search", self.base_path);
        if url.scheme() != self.base_url.scheme()
            || !url.host_str().is_some_and(|host| host.eq_ignore_ascii_case(&self.host))
            || url.port() != self.port
            || !(url.path() == expected_path || url.path() == "/rest/api/content/search")
            || url.fragment().is_some()
            || url.username() != ""
            || url.password().is_some()
        {
            return Err(contract("Confluence search continuation leaves the configured search endpoint"));
        }
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let expected_cql = search_cql(space_key);
        let mut cql = None;
        let mut found_limit = None;
        let mut found_expand = None;
        let mut continuation = None;
        for (name, value) in pairs {
            match name.as_str() {
                "cql" if cql.is_none() => cql = Some(value),
                "limit" if found_limit.is_none() => found_limit = Some(value),
                "expand" if found_expand.is_none() => found_expand = Some(value),
                "cursor" if continuation.is_none() => continuation = Some(SearchPage::Cursor(value)),
                "start" if continuation.is_none() => {
                    let start = value.parse::<u64>().ok()
                        .filter(|start| start.to_string() == value)
                        .ok_or_else(|| contract("Confluence search offset is malformed"))?;
                    continuation = Some(SearchPage::Start(start));
                }
                _ => return Err(contract("Confluence search continuation has altered query parameters")),
            }
        }
        if cql.as_deref() != Some(expected_cql.as_str())
            || found_limit.as_deref() != Some(limit.to_string().as_str())
            || found_expand.as_deref() != Some(expand)
        {
            return Err(contract("Confluence search continuation has altered query parameters"));
        }
        let continuation = continuation
            .ok_or_else(|| contract("Confluence search continuation has no cursor or offset"))?;
        if let SearchPage::Cursor(cursor) = &continuation
            && !cursor_valid(cursor)
        {
            return Err(contract("Confluence search cursor is malformed"));
        }
        Ok(Some(continuation))
    }

    fn listing_page(item: &Value, space_key: &str) -> Result<SpacePage, InspectionError> {
        let page_id = id_text(item.get("id"))
            .filter(|id| page_id_valid(id))
            .ok_or_else(|| contract("Confluence search result has an invalid page id"))?;
        if item.get("type").and_then(Value::as_str).is_some_and(|kind| kind != "page")
            || item.pointer("/space/key").and_then(Value::as_str).is_some_and(|key| key != space_key)
        {
            return Err(contract("Confluence search returned a page outside the requested space"));
        }
        let title = item.get("title").and_then(Value::as_str)
            .filter(|title| title_valid(title))
            .ok_or_else(|| contract("Confluence search result has an invalid title"))?
            .to_owned();
        let version = item.pointer("/version/number").and_then(Value::as_u64)
            .or_else(|| item.get("version").and_then(Value::as_u64))
            .ok_or_else(|| contract("Confluence search result has an invalid version"))?;
        let ancestors = item.get("ancestors").and_then(Value::as_array)
            .ok_or_else(|| contract("Confluence search result has invalid ancestors"))?
            .iter().map(|ancestor| {
                id_text(ancestor.get("id")).filter(|id| page_id_valid(id))
                    .ok_or_else(|| contract("Confluence search result has an invalid ancestor id"))
            }).collect::<Result<Vec<_>, _>>()?;
        let position = item.get("position").and_then(Value::as_i64)
            .or_else(|| item.pointer("/extensions/position").and_then(Value::as_i64));
        Ok(SpacePage { page_id, title, version, ancestors, position })
    }

    fn check_authority(&self, request: &SourceFetchRequest) -> Result<(), InspectionError> {
        let authority = &request.authority;
        if request.provider_id != self.provider_id
            || authority.provider_instance != self.instance
            || !authority.origin_host.eq_ignore_ascii_case(&self.host)
            || authority.origin_port != self.port
            || authority.origin_base_path.trim_end_matches('/') != self.base_path
            || !authority.owner.is_empty()
            || !authority.repository.is_empty()
        {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Confluence request does not match the configured Confluence instance",
            ));
        }
        Ok(())
    }

    /// `Info` is the instance proof: the id must echo and the page URL must
    /// lie inside the configured instance.
    async fn info(&self, page_id: &str) -> Result<PageInfo, InspectionError> {
        let value = self
            .json(&ConfluenceCall::Info {
                page_id: page_id.into(),
            })
            .await?;
        if id_text(value.get("id")).as_deref() != Some(page_id) {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Confluence returned a different page",
            ));
        }
        let url = value
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| self.url_in_instance(url))
            .ok_or_else(|| {
                InspectionError::new(
                    "source_identity_mismatch",
                    "Confluence CLI is connected to a different site than the configured provider",
                )
            })?;
        if value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "page")
        {
            return Err(InspectionError::new(
                "source_capability_unavailable",
                "only Confluence pages can be added",
            ));
        }
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| title_valid(title))
            .ok_or_else(|| contract("Confluence page title is missing or malformed"))?;
        let space_key = value
            .get("spaceKey")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/space/key").and_then(Value::as_str))
            .filter(|key| space_key_valid(key))
            .ok_or_else(|| contract("Confluence page has no valid space key"))?;
        let version = match value.get("version") {
            None | Some(Value::Null) => None,
            Some(version) => Some(
                version
                    .as_u64()
                    .or_else(|| version.get("number").and_then(Value::as_u64))
                    .ok_or_else(|| contract("Confluence page version is malformed"))?,
            ),
        };
        Ok(PageInfo {
            page_id: page_id.into(),
            title: title.into(),
            space_key: space_key.into(),
            version,
            url: url.into(),
        })
    }

    fn page(&self, info: PageInfo) -> ConfluencePage {
        ConfluencePage {
            canonical_url: confluence_page_url(&self.instance, &info.page_id),
            page_id: info.page_id,
            space_key: info.space_key,
            title: info.title,
            version: info.version,
            source_url: info.url,
        }
    }

    /// DC `/display/<KEY>/<title>`: `Find`, then `Info` must agree on
    /// space and title.
    async fn find(&self, space_key: &str, title: &str) -> Result<PageInfo, InspectionError> {
        let found = self
            .json(&ConfluenceCall::Find {
                space_key: space_key.into(),
                title: title.into(),
            })
            .await?;
        let not_found =
            || InspectionError::new("source_not_found", "Confluence page was not found");
        let page_id = id_text(found.get("id"))
            .filter(|id| page_id_valid(id))
            .ok_or_else(not_found)?;
        let info = self.info(&page_id).await.map_err(|error| {
            if error.code == "source_capability_unavailable" {
                not_found()
            } else {
                error
            }
        })?;
        if info.space_key != space_key || info.title != title {
            return Err(not_found());
        }
        Ok(info)
    }

    fn fields_and_container(
        &self,
        info: &PageInfo,
        content: &Value,
        labels: &[String],
    ) -> (Vec<FrontmatterField>, SourceContainer) {
        let space_name = bounded_field(content.pointer("/space/name").and_then(Value::as_str));
        let ancestors: Vec<(String, String)> = content
            .get("ancestors")
            .and_then(Value::as_array)
            .map(|ancestors| {
                ancestors
                    .iter()
                    .filter_map(|ancestor| {
                        let id = id_text(ancestor.get("id")).filter(|id| page_id_valid(id))?;
                        let title = bounded_field(ancestor.get("title").and_then(Value::as_str))
                            .unwrap_or_else(|| id.clone());
                        Some((id, title))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let last_modified = bounded_field(
            content
                .pointer("/history/lastUpdated/when")
                .or_else(|| content.pointer("/version/when"))
                .and_then(Value::as_str),
        );
        // A display name only; account ids and e-mail addresses are never read.
        let person = |pointer: &str| {
            content.pointer(pointer).and_then(|by| {
                bounded_field(
                    by.get("displayName")
                        .or_else(|| by.get("publicName"))
                        .and_then(Value::as_str),
                )
            })
        };
        let last_modified_by = person("/history/lastUpdated/by").or_else(|| person("/version/by"));
        let text = |key: &str, value: String| FrontmatterField {
            key: key.into(),
            value: FrontmatterValue::String(value),
        };
        let mut fields = vec![text("space_key", info.space_key.clone())];
        if let Some(name) = &space_name {
            fields.push(text("space_name", name.clone()));
        }
        fields.push(text("page_id", info.page_id.clone()));
        if let Some((parent, _)) = ancestors.last() {
            fields.push(text("parent_id", parent.clone()));
        }
        if !ancestors.is_empty() {
            fields.push(FrontmatterField {
                key: "ancestors".into(),
                value: FrontmatterValue::Strings(
                    ancestors.iter().map(|(_, title)| title.clone()).collect(),
                ),
            });
            fields.push(FrontmatterField {
                key: "ancestor_ids".into(),
                value: FrontmatterValue::Strings(
                    ancestors.iter().map(|(id, _)| id.clone()).collect(),
                ),
            });
        }
        if let Some(version) = info.version.and_then(|version| i64::try_from(version).ok()) {
            fields.push(FrontmatterField {
                key: "version".into(),
                value: FrontmatterValue::Number(version),
            });
        }
        if let Some(value) = last_modified {
            fields.push(text("last_modified", value));
        }
        if let Some(value) = last_modified_by {
            fields.push(text("last_modified_by", value));
        }
        if !labels.is_empty() {
            fields.push(FrontmatterField {
                key: "labels".into(),
                value: FrontmatterValue::Strings(labels.to_vec()),
            });
        }
        let label = match &space_name {
            Some(name) => format!("{} · {name}", info.space_key),
            None => info.space_key.clone(),
        };
        (
            fields,
            SourceContainer {
                id: info.space_key.clone(),
                label,
            },
        )
    }

    fn attachments(&self, value: &Value) -> Result<(Vec<SourceAttachment>, bool), InspectionError> {
        let list = value
            .get("attachments")
            .and_then(Value::as_array)
            .ok_or_else(|| contract("Confluence attachments output lacks an attachment list"))?;
        let mut attachments = Vec::new();
        for item in list.iter().take(MAX_ATTACHMENTS) {
            let id = id_text(item.get("id"))
                .filter(|id| bounded_field(Some(id)).as_deref() == Some(id.as_str()))
                .ok_or_else(|| contract("Confluence attachment id is malformed"))?;
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| title_valid(title))
                .ok_or_else(|| contract("Confluence attachment title is malformed"))?;
            attachments.push(SourceAttachment {
                id,
                title: title.into(),
                media_type: bounded_field(item.get("mediaType").and_then(Value::as_str)),
                size: item
                    .get("fileSize")
                    .and_then(Value::as_u64)
                    .filter(|size| *size > 0),
                source_url: item
                    .get("downloadLink")
                    .and_then(Value::as_str)
                    .filter(|url| self.url_in_instance(url))
                    .map(str::to_owned),
                source_revision: item
                    .get("version")
                    .and_then(Value::as_u64)
                    .map(|version| version.to_string()),
                path: None,
                not_downloaded: Some(NOT_DOWNLOADED.into()),
            });
        }
        Ok((attachments, list.len() <= MAX_ATTACHMENTS))
    }

    async fn labels(&self, page_id: &str) -> Result<(Vec<String>, bool), InspectionError> {
        let value = self
            .json(&ConfluenceCall::Api(ConfluenceApi::Labels {
                page_id: page_id.into(),
            }))
            .await?;
        let results = value
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| contract("Confluence labels output lacks results"))?;
        let labels = results
            .iter()
            .filter_map(|label| bounded_field(label.get("name").and_then(Value::as_str)))
            .take(256)
            .collect::<Vec<_>>();
        let complete = value.pointer("/_links/next").is_none() && results.len() <= 256;
        Ok((labels, complete))
    }

    fn requested_page(&self, request: &SourceFetchRequest) -> Result<String, InspectionError> {
        self.check_authority(request)?;
        match parse_confluence_input(&self.base_url, &request.artifact_url)? {
            ConfluenceInput::Page { page_id } if request.artifact_url.starts_with("http") => {
                Ok(page_id)
            }
            _ => Err(InspectionError::new(
                "library_input_unrecognized",
                "Confluence fetches need a page URL carrying the page id",
            )),
        }
    }
}

#[async_trait]
impl SourceProvider for ConfluenceSourceProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![SourceCapability::Wiki]
    }
    async fn list_spaces(&self) -> Result<Vec<SpaceSummary>, InspectionError> {
        let value = self.json(&ConfluenceCall::Spaces).await?;
        let spaces = value.get("spaces").and_then(Value::as_array)
            .or_else(|| value.as_array())
            .ok_or_else(|| contract("Confluence spaces output lacks a space list"))?;
        spaces.iter().map(|space| {
            let key = space.get("key").and_then(Value::as_str)
                .filter(|key| space_key_valid(key))
                .ok_or_else(|| contract("Confluence space key is missing or malformed"))?;
            let name = space.get("name").and_then(Value::as_str)
                .filter(|name| bounded_field(Some(name)).as_deref() == Some(*name))
                .ok_or_else(|| contract("Confluence space name is missing or malformed"))?;
            Ok(SpaceSummary { key: key.to_owned(), name: name.to_owned() })
        }).collect()
    }

    async fn list_space_pages(
        &self,
        space_key: &str,
        max_pages: u32,
        cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        if !space_key_valid(space_key) {
            return Err(contract("Confluence space key is malformed"));
        }
        let space = self.json(&ConfluenceCall::Api(ConfluenceApi::Space {
            space_key: space_key.to_owned(),
        })).await?;
        if space.get("key").and_then(Value::as_str) != Some(space_key) {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Confluence returned a different space",
            ));
        }
        let space_name = space.get("name").and_then(Value::as_str)
            .filter(|name| bounded_field(Some(name)).as_deref() == Some(*name))
            .ok_or_else(|| contract("Confluence space name is missing or malformed"))?
            .to_owned();
        let homepage_id = id_text(space.pointer("/homepage/id"))
            .filter(|id| page_id_valid(id));
        let expand = vec![ContentExpand::Version, ContentExpand::Ancestors, ContentExpand::Space];
        let expand_text = expand_value(&expand)?;
        let mut pages = Vec::new();
        let mut total = None;
        let mut continuation = None;
        let mut complete = false;
        while (pages.len() as u64) < u64::from(max_pages) {
            if cancel.load(Ordering::Relaxed) { break; }
            let remaining = (u64::from(max_pages) - pages.len() as u64).min(100) as u8;
            let result = self.json(&ConfluenceCall::Api(ConfluenceApi::Search {
                space_key: space_key.to_owned(),
                limit: remaining,
                expand: expand.clone(),
                page: continuation.take(),
            })).await?;
            let results = result.get("results").and_then(Value::as_array)
                .ok_or_else(|| contract("Confluence page search output lacks results"))?;
            if total.is_none() {
                total = result.get("totalSize").or_else(|| result.get("total"))
                    .and_then(Value::as_u64);
            }
            for result_page in results {
                if pages.len() as u64 >= u64::from(max_pages) { break; }
                pages.push(Self::listing_page(result_page, space_key)?);
            }
            let next = self.search_continuation(&result, space_key, remaining, &expand_text)?;
            match next {
                Some(next) if !results.is_empty() => continuation = Some(next),
                Some(_) => return Err(contract("Confluence search continuation has an empty page")),
                None => {
                    complete = total.is_none_or(|total| pages.len() as u64 >= total);
                    break;
                }
            }
        }
        if cancel.load(Ordering::Relaxed)
            || (pages.len() as u64) >= u64::from(max_pages) && !complete
        {
            complete = false;
        }
        Ok(SpacePageListing { space_name, homepage_id, pages, total, complete })
    }

    async fn page_space(&self, page_id: &str) -> Result<Option<String>, InspectionError> {
        if !page_id_valid(page_id) {
            return Err(contract("Confluence page id must be 1–20 digits"));
        }
        match self.info(page_id).await {
            Ok(info) => Ok(Some(info.space_key)),
            Err(error) if error.code == "source_not_found" => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn metadata(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        let page_id = self.requested_page(request)?;
        let info = self.info(&page_id).await?;
        Ok(SourceMetadata {
            title: info.title,
            source_branch: None,
            source_url: Some(info.url),
            source_commit: None,
            description: None,
        })
    }

    async fn resolve_input(&self, input: &str) -> Result<ProviderResolution, InspectionError> {
        let info = match parse_confluence_input(&self.base_url, input)? {
            ConfluenceInput::Space { space_key } => {
                return Ok(ProviderResolution::ConfluenceSpace { space_key });
            }
            ConfluenceInput::Page { page_id } => self.info(&page_id).await?,
            ConfluenceInput::Display { space_key, title } => self.find(&space_key, &title).await?,
        };
        Ok(ProviderResolution::ConfluencePage(self.page(info)))
    }

    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let page_id = self.requested_page(request)?;
        let info = self.info(&page_id).await?;
        let content = self
            .json(&ConfluenceCall::Api(ConfluenceApi::Content {
                page_id: page_id.clone(),
                expand: vec![
                    ContentExpand::Ancestors,
                    ContentExpand::Version,
                    ContentExpand::Space,
                    ContentExpand::HistoryLastUpdated,
                ],
            }))
            .await?;
        if id_text(content.get("id")).as_deref() != Some(page_id.as_str()) {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Confluence returned a different page",
            ));
        }
        let (labels, labels_complete) = self.labels(&page_id).await?;
        let body = self
            .run(
                &ConfluenceCall::Read {
                    page_id: page_id.clone(),
                },
                MAX_BODY_BYTES + 1,
            )
            .await?;
        let mut body = String::from_utf8(body)
            .map_err(|_| contract("Confluence page Markdown is not UTF-8"))?;
        if body.ends_with('\n') {
            body.pop();
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(InspectionError::new(
                "source_truncated",
                "Confluence page exceeds Cockpit's source body limit",
            ));
        }
        let attachment_list = self
            .json(&ConfluenceCall::Attachments {
                page_id: page_id.clone(),
            })
            .await?;
        let (attachments, attachments_complete) = self.attachments(&attachment_list)?;
        let mut diagnostics = Vec::new();
        if !labels_complete {
            diagnostics.push(ProjectDiagnostic {
                code: "source_labels_partial".into(),
                message: "Confluence returned only the first page of labels".into(),
                path: None,
            });
        }
        if !attachments_complete {
            diagnostics.push(ProjectDiagnostic {
                code: "source_attachments_partial".into(),
                message: format!("only the first {MAX_ATTACHMENTS} attachments are listed"),
                path: None,
            });
        }
        let (fields, container) = self.fields_and_container(&info, &content, &labels);
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: self.instance.clone(),
                resource_type: "page".into(),
                canonical_id: page_id,
            },
            title: info.title.clone(),
            source_url: Some(info.url.clone()),
            original_url: None,
            source_revision: info.version.map(|version| version.to_string()),
            complete: true,
            diagnostics,
            body,
            container: Some(container),
            fields,
            attachments,
        }])
    }

    async fn download_attachment(
        &self,
        page_id: &str,
        attachment: &AttachmentRef,
        siblings: &[AttachmentRef],
        dest: &Dir,
        dest_path: &Path,
        budget: StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        self.download_attachment_file(page_id, attachment, siblings, dest, dest_path, budget)
            .await
    }
}

#[cfg(test)]
mod tests;
