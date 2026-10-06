//! Read-only Confluence Cloud v2 and Data Center v1 through Cockpit's HTTP client.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use cap_std::fs::Dir;
use cockpit_core::InspectionError;
use cockpit_core::credentials::ProviderCredentials;
use cockpit_core::process::StagingBudget;
use cockpit_core::sources::{
    AttachmentRef, ConfluencePage, DownloadedAttachment, FrontmatterField, FrontmatterValue,
    ProviderResolution, SourceAsset, SourceAttachment, SourceContainer, SourceFetchRequest,
    SourceMetadata, SourceProvider, SourceRef, SpacePage, SpacePageListing, SpaceSummary,
    confluence_page_url,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::projects::{ProviderDeployment, ProviderKind};
use cockpit_protocol::sources::SourceCapability;
use serde_json::Value;
use url::Url;

use crate::confluence_storage::{StorageContext, storage_to_markdown};
use crate::site_http::{DOWNLOADED_NAME, MAX_JSON_BYTES, Service, SiteHttp};

const MAX_ATTACHMENTS: usize = 256;
const MAX_TITLE_CHARS: usize = 255;
const MAX_FIELD_CHARS: usize = 256;
const MAX_SPACES: usize = 10_000;

fn contract(message: &str) -> InspectionError {
    InspectionError::new("source_provider_contract", message)
}
fn identity(message: &str) -> InspectionError {
    InspectionError::new("source_identity_mismatch", message)
}
fn page_id_valid(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}
fn attachment_id_valid(value: &str) -> bool {
    page_id_valid(value.strip_prefix("att").unwrap_or(value))
}
fn space_key_valid(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'~'))
}
fn title_valid(value: &str) -> bool {
    (1..=MAX_TITLE_CHARS).contains(&value.chars().count()) && !value.chars().any(char::is_control)
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
fn required_id(value: Option<&Value>) -> Result<String, InspectionError> {
    id_text(value)
        .filter(|id| page_id_valid(id))
        .ok_or_else(|| contract("Confluence returned an invalid content id"))
}
fn results(value: &Value) -> Result<&[Value], InspectionError> {
    value
        .get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| contract("Confluence response has no results list"))
}
fn required_title(value: &Value) -> Result<String, InspectionError> {
    value
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| title_valid(title))
        .map(str::to_owned)
        .ok_or_else(|| contract("Confluence title is missing or malformed"))
}
fn take_storage(value: &mut Value) -> Option<String> {
    match value.pointer_mut("/body/storage/value")?.take() {
        Value::String(storage) => Some(storage),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfluenceInput {
    Page {
        page_id: String,
    },
    /// A display URL is resolved by its exact space and title.
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
        && url
            .host_str()
            .zip(base.host_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        && url.port_or_known_default() == base.port_or_known_default()
        && (base_path.is_empty()
            || url.path() == base_path
            || url.path().starts_with(&format!("{base_path}/")))
}
/// Recognize a Confluence input for one configured instance without network access.
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
        return Err(identity(
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

#[derive(Debug)]
struct PageRecord {
    page_id: String,
    title: String,
    space_key: String,
    space_name: Option<String>,
    version: Option<u64>,
    last_modified: Option<String>,
    last_modified_by: Option<String>,
    ancestors: Vec<(String, String)>,
    url: String,
    storage: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Continuation {
    Cursor(String),
    Start(u64),
}
#[derive(Debug)]
pub struct ConfluenceSourceProvider {
    provider_id: String,
    base_url: Url,
    instance: String,
    host: String,
    port: Option<u16>,
    base_path: String,
    deployment: ProviderDeployment,
    http: SiteHttp,
}

impl ConfluenceSourceProvider {
    pub fn configured(
        configuration: &ProjectConfiguration,
        provider_id: &str,
        credentials: Arc<ProviderCredentials>,
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
                "Confluence requires a deployment and a credential-free HTTP(S) base URL",
            )
        };
        if provider.kind != ProviderKind::Confluence {
            return Err(invalid());
        }
        let deployment = provider.deployment.ok_or_else(invalid)?;
        let base_url = Url::parse(&provider.base_url).map_err(|_| invalid())?;
        if !matches!(base_url.scheme(), "http" | "https")
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || deployment == ProviderDeployment::Cloud && base_url.path() != "/wiki"
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
        let http = SiteHttp::new(
            base_url.clone(),
            &provider.id,
            Service::Confluence,
            Duration::from_millis(configuration.limits.operation_timeout_ms.into()),
            credentials,
        );
        Ok(Self {
            provider_id: provider.id.clone(),
            base_url,
            instance,
            host,
            port,
            base_path,
            deployment,
            http,
        })
    }
    fn cloud(&self) -> bool {
        self.deployment == ProviderDeployment::Cloud
    }
    fn endpoint(&self, tail: &[&str], query: &[(&str, &str)]) -> Url {
        let mut segments = if self.cloud() {
            vec!["api", "v2"]
        } else {
            vec!["rest", "api"]
        };
        segments.extend_from_slice(tail);
        self.http.endpoint(&segments, query)
    }
    async fn json(&self, tail: &[&str], query: &[(&str, &str)]) -> Result<Value, InspectionError> {
        self.http
            .get_json(self.endpoint(tail, query), MAX_JSON_BYTES)
            .await
            .map_err(Into::into)
    }
    /// Validate a server continuation, retaining only the cursor/offset. All requests
    /// are rebuilt from the trusted endpoint and original filters.
    fn continuation(
        &self,
        value: &Value,
        tail: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Option<Continuation>, InspectionError> {
        let Some(next) = value.pointer("/_links/next").filter(|next| !next.is_null()) else {
            return Ok(None);
        };
        let link = next
            .as_str()
            .filter(|link| !link.is_empty())
            .ok_or_else(|| contract("Confluence returned a malformed continuation"))?;
        let url = self.http.link(link).map_err(InspectionError::from)?;
        if url.path() != self.endpoint(tail, &[]).path() || url.fragment().is_some() {
            return Err(contract(
                "Confluence continuation changed the requested endpoint",
            ));
        }
        let token_key = if self.cloud() { "cursor" } else { "start" };
        let mut token = None;
        let mut names = BTreeSet::new();
        for (key, val) in url.query_pairs() {
            if !names.insert(key.to_string()) {
                return Err(contract("Confluence continuation repeated a parameter"));
            }
            if key == token_key {
                if val.is_empty() || val.len() > 4096 || val.chars().any(char::is_control) {
                    return Err(contract("Confluence continuation is malformed"));
                }
                token =
                    Some(if self.cloud() {
                        Continuation::Cursor(val.into_owned())
                    } else {
                        Continuation::Start(val.parse().map_err(|_| {
                            contract("Confluence continuation has an invalid offset")
                        })?)
                    });
            } else if !query.iter().any(|(expected_key, expected_value)| {
                *expected_key == key && *expected_value == val
            }) {
                return Err(contract(
                    "Confluence continuation changed the requested filters",
                ));
            }
        }
        token
            .map(Some)
            .ok_or_else(|| contract("Confluence continuation has no cursor or offset"))
    }
    async fn paged_json(
        &self,
        tail: &[&str],
        query: &[(&str, &str)],
        next: Option<&Continuation>,
    ) -> Result<Value, InspectionError> {
        let mut pairs = query
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>();
        match next {
            Some(Continuation::Cursor(cursor)) => pairs.push(("cursor".into(), cursor.clone())),
            Some(Continuation::Start(start)) => pairs.push(("start".into(), start.to_string())),
            None if !self.cloud() => pairs.push(("start".into(), "0".into())),
            None => {}
        }
        let borrowed = pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        self.json(tail, &borrowed).await
    }
    fn check_next(
        next: &Option<Continuation>,
        seen: &mut BTreeSet<Continuation>,
        rows: usize,
    ) -> Result<(), InspectionError> {
        if let Some(next) = next {
            if let Continuation::Start(start) = next {
                let previous = match seen.last() {
                    Some(Continuation::Start(value)) => *value,
                    _ => 0,
                };
                if previous.checked_add(rows as u64) != Some(*start) {
                    return Err(contract(
                        "Confluence pagination skipped or repeated an offset",
                    ));
                }
            }
            if rows == 0 || matches!(next, Continuation::Start(0)) || !seen.insert(next.clone()) {
                return Err(contract("Confluence pagination failed to advance"));
            }
        }
        Ok(())
    }
    fn web_url(&self, value: &Value) -> Result<String, InspectionError> {
        let webui = value
            .pointer("/_links/webui")
            .and_then(Value::as_str)
            .filter(|link| !link.is_empty())
            .ok_or_else(|| contract("Confluence page has no web URL"))?;
        let url = self
            .http
            .link(webui)
            .map_err(|_| identity("Confluence answered for a different site"))?;
        if url.fragment().is_some() {
            return Err(identity("Confluence answered with an invalid page URL"));
        }
        Ok(url.to_string())
    }
    fn dc_record(
        &self,
        value: &Value,
        expected_id: Option<&str>,
        full: bool,
    ) -> Result<PageRecord, InspectionError> {
        let page_id = required_id(value.get("id"))?;
        if expected_id.is_some_and(|expected| expected != page_id) {
            return Err(identity("Confluence returned a different page"));
        }
        if value.get("type").and_then(Value::as_str) != Some("page") {
            return Err(InspectionError::new(
                "source_capability_unavailable",
                "only Confluence pages can be added",
            ));
        }
        let space_key = value
            .pointer("/space/key")
            .and_then(Value::as_str)
            .filter(|key| space_key_valid(key))
            .ok_or_else(|| contract("Confluence page has no valid space key"))?
            .to_owned();
        let ancestors = if full {
            value
                .get("ancestors")
                .and_then(Value::as_array)
                .ok_or_else(|| contract("Confluence page has no ancestors list"))?
                .iter()
                .map(|ancestor| {
                    let id = required_id(ancestor.get("id"))?;
                    let title = bounded_field(ancestor.get("title").and_then(Value::as_str))
                        .unwrap_or_else(|| id.clone());
                    Ok((id, title))
                })
                .collect::<Result<Vec<_>, InspectionError>>()?
        } else {
            Vec::new()
        };
        let person = |pointer| {
            value.pointer(pointer).and_then(|person: &Value| {
                bounded_field(
                    person
                        .get("displayName")
                        .or_else(|| person.get("publicName"))
                        .and_then(Value::as_str),
                )
            })
        };
        Ok(PageRecord {
            page_id,
            title: required_title(value)?,
            space_key,
            space_name: bounded_field(value.pointer("/space/name").and_then(Value::as_str)),
            version: value.pointer("/version/number").and_then(Value::as_u64),
            last_modified: bounded_field(
                value
                    .pointer("/history/lastUpdated/when")
                    .or_else(|| value.pointer("/version/when"))
                    .and_then(Value::as_str),
            ),
            last_modified_by: person("/history/lastUpdated/by").or_else(|| person("/version/by")),
            ancestors,
            url: self.web_url(value)?,
            storage: None,
        })
    }
    async fn cloud_space(&self, id: &str) -> Result<Value, InspectionError> {
        let value = self.json(&["spaces", id], &[]).await?;
        if id_text(value.get("id")).as_deref() != Some(id) {
            return Err(identity("Confluence returned a different space"));
        }
        Self::space_summary(&value)?;
        Ok(value)
    }
    async fn space_by_key(&self, key: &str) -> Result<Value, InspectionError> {
        if !space_key_valid(key) {
            return Err(contract("Confluence space key is malformed"));
        }
        let value = if self.cloud() {
            let found = self.json(&["spaces"], &[("keys", key)]).await?;
            let rows = results(&found)?;
            if rows.len() != 1
                || self
                    .continuation(&found, &["spaces"], &[("keys", key)])?
                    .is_some()
            {
                return Err(InspectionError::new(
                    "source_not_found",
                    "Confluence space does not exist or is not visible to the stored token",
                ));
            }
            rows[0].clone()
        } else {
            self.json(&["space", key], &[("expand", "homepage")])
                .await?
        };
        if value.get("key").and_then(Value::as_str) != Some(key) {
            return Err(identity("Confluence returned a different space"));
        }
        Self::space_summary(&value)?;
        Ok(value)
    }
    fn cloud_record(
        &self,
        value: &Value,
        space: &Value,
        expected_id: Option<&str>,
    ) -> Result<PageRecord, InspectionError> {
        let page_id = required_id(value.get("id"))?;
        if expected_id.is_some_and(|id| id != page_id) {
            return Err(identity("Confluence returned a different page"));
        }
        if required_id(value.get("spaceId"))? != required_id(space.get("id"))? {
            return Err(identity("Confluence returned a page in a different space"));
        }
        let summary = Self::space_summary(space)?;
        Ok(PageRecord {
            page_id,
            title: required_title(value)?,
            space_key: summary.key,
            space_name: Some(summary.name),
            version: value.pointer("/version/number").and_then(Value::as_u64),
            last_modified: bounded_field(
                value.pointer("/version/createdAt").and_then(Value::as_str),
            ),
            last_modified_by: None,
            ancestors: Vec::new(),
            url: self.web_url(value)?,
            storage: None,
        })
    }
    async fn info(&self, page_id: &str) -> Result<PageRecord, InspectionError> {
        if !page_id_valid(page_id) {
            return Err(contract("Confluence page id must be 1–20 digits"));
        }
        if self.cloud() {
            let value = self.json(&["pages", page_id], &[]).await?;
            if id_text(value.get("id")).as_deref() != Some(page_id) {
                return Err(identity("Confluence returned a different page"));
            }
            let space_id = required_id(value.get("spaceId"))?;
            let space = self.cloud_space(&space_id).await?;
            self.cloud_record(&value, &space, Some(page_id))
        } else {
            let value = self
                .json(&["content", page_id], &[("expand", "space,version")])
                .await?;
            self.dc_record(&value, Some(page_id), false)
        }
    }
    fn page(&self, info: PageRecord) -> ConfluencePage {
        ConfluencePage {
            canonical_url: confluence_page_url(&self.instance, &info.page_id),
            page_id: info.page_id,
            title: info.title,
            space_key: info.space_key,
            version: info.version,
            source_url: info.url,
        }
    }
    async fn find(&self, space_key: &str, title: &str) -> Result<PageRecord, InspectionError> {
        let not_found = || {
            InspectionError::new(
                "source_not_found",
                "Confluence page does not exist or is not visible to the stored token",
            )
        };
        let record = if self.cloud() {
            let space = self.space_by_key(space_key).await?;
            let id = required_id(space.get("id"))?;
            let query = [
                ("space-id", id.as_str()),
                ("title", title),
                ("status", "current"),
                ("limit", "2"),
            ];
            let value = self.json(&["pages"], &query).await?;
            let rows = results(&value)?;
            if rows.len() != 1 || self.continuation(&value, &["pages"], &query)?.is_some() {
                return Err(not_found());
            }
            self.cloud_record(&rows[0], &space, None)?
        } else {
            let query = [
                ("spaceKey", space_key),
                ("title", title),
                ("type", "page"),
                ("expand", "space,version"),
                ("limit", "2"),
            ];
            let value = self.json(&["content"], &query).await?;
            let rows = results(&value)?;
            if rows.len() != 1 || self.continuation(&value, &["content"], &query)?.is_some() {
                return Err(not_found());
            }
            self.dc_record(&rows[0], None, false)?
        };
        if record.title != title || record.space_key != space_key {
            return Err(not_found());
        }
        Ok(record)
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
            return Err(identity(
                "Confluence request does not match the configured Confluence instance",
            ));
        }
        Ok(())
    }
    fn requested_page(&self, request: &SourceFetchRequest) -> Result<String, InspectionError> {
        self.check_authority(request)?;
        match parse_confluence_input(&self.base_url, &request.artifact_url)? {
            ConfluenceInput::Page { page_id } if request.artifact_url.starts_with("http") => {
                Ok(page_id)
            }
            _ => Err(contract("Confluence fetch requires an authorized page URL")),
        }
    }
    fn fields_and_container(
        &self,
        info: &PageRecord,
        labels: &[String],
    ) -> (Vec<FrontmatterField>, SourceContainer) {
        let text = |key: &str, value: String| FrontmatterField {
            key: key.into(),
            value: FrontmatterValue::String(value),
        };
        let mut fields = vec![text("space_key", info.space_key.clone())];
        if let Some(name) = &info.space_name {
            fields.push(text("space_name", name.clone()));
        }
        fields.push(text("page_id", info.page_id.clone()));
        if let Some((parent, _)) = info.ancestors.last() {
            fields.push(text("parent_id", parent.clone()));
        }
        if !info.ancestors.is_empty() {
            fields.push(FrontmatterField {
                key: "ancestors".into(),
                value: FrontmatterValue::Strings(
                    info.ancestors
                        .iter()
                        .map(|(_, title)| title.clone())
                        .collect(),
                ),
            });
            fields.push(FrontmatterField {
                key: "ancestor_ids".into(),
                value: FrontmatterValue::Strings(
                    info.ancestors.iter().map(|(id, _)| id.clone()).collect(),
                ),
            });
        }
        if let Some(version) = info.version.and_then(|version| i64::try_from(version).ok()) {
            fields.push(FrontmatterField {
                key: "version".into(),
                value: FrontmatterValue::Number(version),
            });
        }
        if let Some(value) = &info.last_modified {
            fields.push(text("last_modified", value.clone()));
        }
        if let Some(value) = &info.last_modified_by {
            fields.push(text("last_modified_by", value.clone()));
        }
        if !labels.is_empty() {
            fields.push(FrontmatterField {
                key: "labels".into(),
                value: FrontmatterValue::Strings(labels.to_vec()),
            });
        }
        let label = match &info.space_name {
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
    async fn labels(&self, id: &str) -> Result<(Vec<String>, bool), InspectionError> {
        let tail = if self.cloud() {
            vec!["pages", id, "labels"]
        } else {
            vec!["content", id, "label"]
        };
        let query = [("limit", if self.cloud() { "250" } else { "200" })];
        let value = self.json(&tail, &query).await?;
        let rows = results(&value)?;
        let complete =
            rows.len() <= MAX_ATTACHMENTS && self.continuation(&value, &tail, &query)?.is_none();
        let mut labels = rows
            .iter()
            .take(MAX_ATTACHMENTS)
            .map(|row| {
                bounded_field(row.get("name").and_then(Value::as_str))
                    .ok_or_else(|| contract("Confluence returned an invalid label"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        labels.sort();
        labels.dedup();
        Ok((labels, complete))
    }
    fn attachment(&self, value: &Value) -> Result<SourceAttachment, InspectionError> {
        let id = id_text(value.get("id"))
            .filter(|id| attachment_id_valid(id))
            .ok_or_else(|| contract("Confluence attachment id is malformed"))?;
        let download = value
            .get("downloadLink")
            .or_else(|| value.pointer("/_links/download"))
            .and_then(Value::as_str);
        let source_url = download
            .map(|link| {
                self.http
                    .link(link)
                    .map(|url| url.to_string())
                    .map_err(InspectionError::from)
            })
            .transpose()?;
        Ok(SourceAttachment {
            id,
            title: required_title(value)?,
            media_type: bounded_field(
                value
                    .get("mediaType")
                    .or_else(|| value.pointer("/metadata/mediaType"))
                    .and_then(Value::as_str),
            ),
            size: value
                .get("fileSize")
                .or_else(|| value.pointer("/extensions/fileSize"))
                .and_then(Value::as_u64),
            source_url,
            source_revision: value
                .pointer("/version/number")
                .and_then(Value::as_u64)
                .map(|v| v.to_string()),
            path: None,
            not_downloaded: Some("not downloaded".into()),
        })
    }
    async fn attachments(
        &self,
        id: &str,
    ) -> Result<(Vec<SourceAttachment>, bool), InspectionError> {
        let tail = if self.cloud() {
            vec!["pages", id, "attachments"]
        } else {
            vec!["content", id, "child", "attachment"]
        };
        let query = if self.cloud() {
            vec![("limit", "250")]
        } else {
            vec![("limit", "200"), ("expand", "version")]
        };
        let mut next = None;
        let mut seen = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut attachments = Vec::new();
        loop {
            let value = self.paged_json(&tail, &query, next.as_ref()).await?;
            let rows = results(&value)?;
            let continuation = self.continuation(&value, &tail, &query)?;
            Self::check_next(&continuation, &mut seen, rows.len())?;
            for row in rows {
                if attachments.len() == MAX_ATTACHMENTS {
                    return Ok((attachments, false));
                }
                let attachment = self.attachment(row)?;
                if !ids.insert(attachment.id.clone()) {
                    return Err(contract("Confluence repeated an attachment id"));
                }
                attachments.push(attachment);
            }
            if continuation.is_none() {
                return Ok((attachments, true));
            }
            if attachments.len() == MAX_ATTACHMENTS {
                return Ok((attachments, false));
            }
            next = continuation;
        }
    }
    async fn ancestor_rows(&self, page_id: &str) -> Result<Vec<Value>, InspectionError> {
        self.ancestor_rows_until_cancelled(page_id, None)
            .await
            .map(|(rows, _)| rows)
    }
    async fn ancestor_rows_until_cancelled(
        &self,
        page_id: &str,
        cancel: Option<&AtomicBool>,
    ) -> Result<(Vec<Value>, bool), InspectionError> {
        let tail = ["pages", page_id, "ancestors"];
        let query = [("limit", "250")];
        let mut rows = Vec::new();
        let mut next = None;
        let mut seen = BTreeSet::new();
        let mut ids = BTreeSet::new();
        loop {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                return Ok((rows, false));
            }
            let value = self.paged_json(&tail, &query, next.as_ref()).await?;
            let page = results(&value)?;
            next = self.continuation(&value, &tail, &query)?;
            Self::check_next(&next, &mut seen, page.len())?;
            for ancestor in page {
                let id = required_id(ancestor.get("id"))?;
                if id == page_id || !ids.insert(id) {
                    return Err(contract("Confluence returned cyclic or repeated ancestors"));
                }
                rows.push(ancestor.clone());
            }
            if rows.len() > MAX_SPACES {
                return Err(InspectionError::new(
                    "source_truncated",
                    "Confluence ancestry exceeds Cockpit's limit",
                ));
            }
            if next.is_none() {
                return Ok((rows, true));
            }
        }
    }
    async fn ancestor_titles(
        &self,
        rows: &[Value],
    ) -> Result<Vec<(String, String)>, InspectionError> {
        let page_ids = rows
            .iter()
            .filter(|row| row.get("type").and_then(Value::as_str) == Some("page"))
            .map(|row| required_id(row.get("id")))
            .collect::<Result<Vec<_>, _>>()?;
        let pages = async {
            let mut titles = BTreeMap::new();
            for chunk in page_ids.chunks(250) {
                let ids = chunk.join(",");
                let query = [("id", ids.as_str()), ("limit", "250")];
                let value = self.json(&["pages"], &query).await?;
                if self.continuation(&value, &["pages"], &query)?.is_some() {
                    return Err(contract("Confluence ancestor title response is incomplete"));
                }
                for page in results(&value)? {
                    let id = required_id(page.get("id"))?;
                    if !chunk.contains(&id) || titles.insert(id, required_title(page)?).is_some() {
                        return Err(contract("Confluence returned an unexpected ancestor page"));
                    }
                }
                if chunk.iter().any(|id| !titles.contains_key(id)) {
                    return Err(contract("Confluence omitted an ancestor page"));
                }
            }
            Ok::<_, InspectionError>(titles)
        };
        let folders = async {
            let mut titles = BTreeMap::new();
            for row in rows
                .iter()
                .filter(|row| row.get("type").and_then(Value::as_str) == Some("folder"))
            {
                let id = required_id(row.get("id"))?;
                let folder = self.json(&["folders", &id], &[]).await?;
                if id_text(folder.get("id")).as_deref() != Some(id.as_str()) {
                    return Err(identity("Confluence returned a different folder"));
                }
                titles.insert(id, required_title(&folder)?);
            }
            Ok::<_, InspectionError>(titles)
        };
        let (mut titles, mut folders) = tokio::try_join!(pages, folders)?;
        let mut ancestors = Vec::with_capacity(rows.len());
        for row in rows {
            let id = required_id(row.get("id"))?;
            let title = match row.get("type").and_then(Value::as_str) {
                Some("page") => titles
                    .remove(&id)
                    .ok_or_else(|| contract("Confluence omitted an ancestor title"))?,
                Some("folder") => folders
                    .remove(&id)
                    .ok_or_else(|| contract("Confluence omitted an ancestor folder"))?,
                _ => id.clone(),
            };
            ancestors.push((id, title));
        }
        Ok(ancestors)
    }
    async fn display_name(&self, account: Option<&str>) -> Result<Option<String>, InspectionError> {
        let Some(account) = account.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let url = self
            .http
            .endpoint(&["rest", "api", "user"], &[("accountId", account)]);
        match self.http.get_json(url, MAX_JSON_BYTES).await {
            Ok(person) => Ok(bounded_field(
                person
                    .get("displayName")
                    .or_else(|| person.get("publicName"))
                    .and_then(Value::as_str),
            )),
            Err(failure)
                if matches!(
                    failure.status,
                    Some(reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND)
                ) =>
            {
                Ok(None)
            }
            Err(failure) => Err(failure.into()),
        }
    }
    async fn fetch_record(
        &self,
        id: &str,
    ) -> Result<(PageRecord, Vec<String>, bool, Vec<SourceAttachment>, bool), InspectionError> {
        if self.cloud() {
            let page_endpoint = ["pages", id];
            let (
                mut content,
                ancestor_rows,
                (labels, labels_complete),
                (attachments, attachments_complete),
            ) = tokio::try_join!(
                self.json(&page_endpoint, &[("body-format", "storage")]),
                self.ancestor_rows(id),
                self.labels(id),
                self.attachments(id)
            )?;
            if id_text(content.get("id")).as_deref() != Some(id) {
                return Err(identity("Confluence returned a different page"));
            }
            let parent = id_text(content.get("parentId"));
            let last_ancestor = ancestor_rows
                .last()
                .map(|row| required_id(row.get("id")))
                .transpose()?;
            if parent != last_ancestor {
                return Err(contract(
                    "Confluence ancestor chain does not match the page parent",
                ));
            }
            let space_id = required_id(content.get("spaceId"))?;
            let (space, ancestors, editor) = tokio::try_join!(
                self.cloud_space(&space_id),
                self.ancestor_titles(&ancestor_rows),
                self.display_name(content.pointer("/version/authorId").and_then(Value::as_str))
            )?;
            let mut record = self.cloud_record(&content, &space, Some(id))?;
            record.ancestors = ancestors;
            record.last_modified_by = editor;
            record.storage = take_storage(&mut content);
            Ok((
                record,
                labels,
                labels_complete,
                attachments,
                attachments_complete,
            ))
        } else {
            let content_endpoint = ["content", id];
            let (mut content, (labels, labels_complete), (attachments, attachments_complete)) = tokio::try_join!(
                self.json(
                    &content_endpoint,
                    &[(
                        "expand",
                        "ancestors,version,space,history.lastUpdated,body.storage"
                    )]
                ),
                self.labels(id),
                self.attachments(id)
            )?;
            let mut record = self.dc_record(&content, Some(id), true)?;
            record.storage = take_storage(&mut content);
            Ok((
                record,
                labels,
                labels_complete,
                attachments,
                attachments_complete,
            ))
        }
    }
    fn space_summary(value: &Value) -> Result<SpaceSummary, InspectionError> {
        let key = value
            .get("key")
            .and_then(Value::as_str)
            .filter(|key| space_key_valid(key))
            .ok_or_else(|| contract("Confluence space key is missing or malformed"))?;
        let name = bounded_field(value.get("name").and_then(Value::as_str))
            .ok_or_else(|| contract("Confluence space name is missing or malformed"))?;
        Ok(SpaceSummary {
            key: key.into(),
            name,
        })
    }
    fn listing_page(value: &Value, key: &str) -> Result<SpacePage, InspectionError> {
        if value.get("type").and_then(Value::as_str) != Some("page")
            || value.pointer("/space/key").and_then(Value::as_str) != Some(key)
        {
            return Err(contract(
                "Confluence search returned a page outside the requested space",
            ));
        }
        let ancestors = value
            .get("ancestors")
            .and_then(Value::as_array)
            .ok_or_else(|| contract("Confluence search result has invalid ancestors"))?
            .iter()
            .map(|ancestor| required_id(ancestor.get("id")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SpacePage {
            page_id: required_id(value.get("id"))?,
            title: required_title(value)?,
            version: value
                .pointer("/version/number")
                .and_then(Value::as_u64)
                .ok_or_else(|| contract("Confluence search result has an invalid version"))?,
            ancestors,
            position: value
                .get("position")
                .or_else(|| value.pointer("/extensions/position"))
                .and_then(Value::as_i64),
        })
    }
    async fn cloud_listing_ancestors(
        &self,
        rows: &[Value],
        pages: &mut Vec<SpacePage>,
        cancel: &AtomicBool,
    ) -> Result<bool, InspectionError> {
        let by_id = rows
            .iter()
            .map(|row| Ok((required_id(row.get("id"))?, row)))
            .collect::<Result<BTreeMap<_, _>, InspectionError>>()?;
        let mut chains: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut parent_chains: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut finished = 0;
        'pages: for page in pages.iter_mut() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut path = Vec::new();
            let mut visiting = BTreeSet::new();
            let mut current = page.page_id.clone();
            let mut chain;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break 'pages;
                }
                if let Some(cached) = chains.get(&current) {
                    chain = cached.clone();
                    break;
                }
                if !visiting.insert(current.clone()) {
                    return Err(contract("Confluence listed a cyclic page hierarchy"));
                }
                let row = by_id
                    .get(&current)
                    .ok_or_else(|| contract("Confluence omitted a listed page"))?;
                let parent = id_text(row.get("parentId"));
                if parent.is_none() {
                    chain = Vec::new();
                    chains.insert(current.clone(), chain.clone());
                    break;
                }
                let parent = parent
                    .filter(|parent| page_id_valid(parent))
                    .ok_or_else(|| contract("Confluence returned an invalid parent id"))?;
                if row.get("parentType").and_then(Value::as_str) == Some("page")
                    && by_id.contains_key(&parent)
                {
                    path.push((current, parent.clone()));
                    current = parent;
                } else {
                    chain = if let Some(cached) = parent_chains.get(&parent) {
                        cached.clone()
                    } else {
                        let (rows, complete) = self
                            .ancestor_rows_until_cancelled(&current, Some(cancel))
                            .await?;
                        if !complete || cancel.load(Ordering::Relaxed) {
                            break 'pages;
                        }
                        let ancestors = rows
                            .iter()
                            .map(|row| required_id(row.get("id")))
                            .collect::<Result<Vec<_>, _>>()?;
                        if ancestors.last() != Some(&parent) {
                            return Err(contract(
                                "Confluence ancestor chain does not match the page parent",
                            ));
                        }
                        parent_chains.insert(parent, ancestors.clone());
                        ancestors
                    };
                    chains.insert(current.clone(), chain.clone());
                    break;
                }
            }
            for (child, parent) in path.into_iter().rev() {
                chain.push(parent);
                chains.insert(child, chain.clone());
            }
            if chain.iter().any(|id| id == &page.page_id) {
                return Err(contract("Confluence listed a cyclic page hierarchy"));
            }
            page.ancestors = chain;
            finished += 1;
        }
        let complete = finished == pages.len();
        pages.truncate(finished);
        Ok(complete)
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
        let tail = if self.cloud() { "spaces" } else { "space" };
        let query = [
            ("limit", if self.cloud() { "250" } else { "200" }),
            ("status", "current"),
        ];
        let mut spaces = BTreeMap::new();
        let mut next = None;
        let mut seen = BTreeSet::new();
        loop {
            let value = self.paged_json(&[tail], &query, next.as_ref()).await?;
            let rows = results(&value)?;
            next = self.continuation(&value, &[tail], &query)?;
            Self::check_next(&next, &mut seen, rows.len())?;
            for row in rows {
                let summary = Self::space_summary(row)?;
                if spaces.insert(summary.key.clone(), summary).is_some() {
                    return Err(contract("Confluence repeated a space key"));
                }
                if spaces.len() > MAX_SPACES {
                    return Err(InspectionError::new(
                        "source_truncated",
                        "Confluence returned more than 10000 spaces",
                    ));
                }
            }
            if next.is_none() {
                return Ok(spaces.into_values().collect());
            }
        }
    }
    async fn list_space_pages(
        &self,
        space_key: &str,
        max_pages: u32,
        cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        let space = self.space_by_key(space_key).await?;
        let summary = Self::space_summary(&space)?;
        let homepage_id = id_text(if self.cloud() {
            space.get("homepageId")
        } else {
            space.pointer("/homepage/id")
        })
        .filter(|id| page_id_valid(id));
        let space_id = if self.cloud() {
            Some(required_id(space.get("id"))?)
        } else {
            None
        };
        let tail = if let Some(id) = &space_id {
            vec!["spaces", id.as_str(), "pages"]
        } else {
            vec!["content", "search"]
        };
        let cql = format!("space=\"{space_key}\" and type=page");
        let query = if self.cloud() {
            vec![("depth", "all"), ("limit", "250"), ("status", "current")]
        } else {
            vec![
                ("cql", cql.as_str()),
                ("limit", "100"),
                ("expand", "version,ancestors,space"),
            ]
        };
        let mut next = None;
        let mut seen = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut pages = Vec::new();
        let mut raw = Vec::new();
        let mut total = None;
        let mut complete = false;
        while pages.len() < max_pages as usize && !cancel.load(Ordering::Relaxed) {
            let value = self.paged_json(&tail, &query, next.as_ref()).await?;
            let rows = results(&value)?;
            if !self.cloud() && total.is_none() {
                total = value
                    .get("totalSize")
                    .or_else(|| value.get("total"))
                    .and_then(Value::as_u64);
            }
            next = self.continuation(&value, &tail, &query)?;
            Self::check_next(&next, &mut seen, rows.len())?;
            let mut capped = false;
            for row in rows {
                if pages.len() == max_pages as usize {
                    capped = true;
                    break;
                }
                let page = if self.cloud() {
                    if required_id(row.get("spaceId"))?.as_str()
                        != space_id.as_deref().unwrap_or_default()
                    {
                        return Err(contract("Confluence listed a page from a different space"));
                    }
                    SpacePage {
                        page_id: required_id(row.get("id"))?,
                        title: required_title(row)?,
                        version: row
                            .pointer("/version/number")
                            .and_then(Value::as_u64)
                            .ok_or_else(|| contract("Confluence page version is malformed"))?,
                        ancestors: Vec::new(),
                        position: row.get("position").and_then(Value::as_i64),
                    }
                } else {
                    Self::listing_page(row, space_key)?
                };
                if !ids.insert(page.page_id.clone()) {
                    return Err(contract("Confluence repeated a page id"));
                }
                pages.push(page);
                if self.cloud() {
                    raw.push(row.clone());
                }
            }
            if next.is_none() {
                complete = !capped && total.is_none_or(|total| pages.len() as u64 >= total);
                break;
            }
        }
        if self.cloud()
            && !self
                .cloud_listing_ancestors(&raw, &mut pages, cancel)
                .await?
        {
            complete = false;
        }
        if cancel.load(Ordering::Relaxed) {
            complete = false;
        }
        pages.sort_by(|left, right| left.page_id.cmp(&right.page_id));
        Ok(SpacePageListing {
            space_name: summary.name,
            homepage_id,
            pages,
            total,
            complete,
        })
    }
    async fn page_space(&self, page_id: &str) -> Result<Option<String>, InspectionError> {
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
        let info = self.info(&self.requested_page(request)?).await?;
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
        let (info, labels, labels_complete, attachments, attachments_complete) =
            self.fetch_record(&page_id).await?;
        let storage = info
            .storage
            .as_deref()
            .ok_or_else(|| contract("Confluence page has no storage body"))?;
        let body = storage_to_markdown(
            storage,
            &StorageContext {
                instance: &self.instance,
                space_key: &info.space_key,
                attachments: &attachments,
            },
        )?;
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
        let (fields, container) = self.fields_and_container(&info, &labels);
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: self.instance.clone(),
                resource_type: "page".into(),
                canonical_id: page_id,
            },
            title: info.title,
            source_url: Some(info.url),
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
    async fn attachment_downloads(&self, resource_type: &str) -> Result<(), InspectionError> {
        if resource_type != "page" {
            return Err(InspectionError::new(
                "source_capability_unavailable",
                "selected source provider does not support this operation",
            ));
        }
        self.http.require_credentials().await
    }
    async fn download_attachment(
        &self,
        page_id: &str,
        attachment: &AttachmentRef,
        _siblings: &[AttachmentRef],
        dest: &Dir,
        _dest_path: &Path,
        budget: StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        if !page_id_valid(page_id)
            || !attachment_id_valid(&attachment.id)
            || !title_valid(&attachment.title)
        {
            return Err(contract("Confluence attachment identity is malformed"));
        }
        let value = if self.cloud() {
            self.json(&["attachments", &attachment.id], &[]).await?
        } else {
            self.json(
                &["content", &attachment.id],
                &[("expand", "container,version")],
            )
            .await?
        };
        let owner = if self.cloud() {
            value.get("pageId")
        } else {
            value.pointer("/container/id")
        };
        if id_text(value.get("id")).as_deref() != Some(attachment.id.as_str())
            || id_text(owner).as_deref() != Some(page_id)
            || value.get("title").and_then(Value::as_str) != Some(&attachment.title)
            || !self.cloud() && value.get("type").and_then(Value::as_str) != Some("attachment")
        {
            return Err(identity("Confluence returned a different attachment"));
        }
        let current = self.attachment(&value)?;
        let source = current
            .source_url
            .ok_or_else(|| contract("Confluence attachment has no download URL"))?;
        if attachment
            .bytes
            .zip(current.size)
            .is_some_and(|(old, new)| old != new)
        {
            return Err(InspectionError::new(
                "source_attachment_size",
                "Confluence attachment size changed",
            ));
        }
        let url = self.http.link(&source).map_err(InspectionError::from)?;
        self.http
            .download(url, budget.bytes, current.size.or(attachment.bytes), dest)
            .await
            .map_err(InspectionError::from)?;
        Ok(DownloadedAttachment {
            attachment_id: attachment.id.clone(),
            file_name: DOWNLOADED_NAME.into(),
        })
    }
}

#[cfg(test)]
mod tests;
