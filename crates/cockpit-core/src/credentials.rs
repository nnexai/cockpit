//! Provider credentials kept in an OS vault behind [`CredentialVault`].
//!
//! Write-only by design: a credential can be set, replaced, cleared and
//! reported as present, and it reaches native HTTP only through
//! [`ProviderCredentials::required`].
//! The status/set/clear API never returns a token or username, and every error
//! message is a fixed string.
//!
//! A vault item is identified by the provider id and the configured site
//! (`instance`), and its secret repeats that instance. Changing a provider's
//! `base_url` therefore never hands an old token to a new site.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cockpit_protocol::credentials::{
    ProviderAuthKind, ProviderCredentialClearRequest, ProviderCredentialSetRequest,
    ProviderCredentialState, ProviderCredentialStatus, ProviderCredentialStatusList,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectProvider};
use serde::{Deserialize, Serialize};

use crate::InspectionError;

/// A vault call, including an unlock prompt, may block up to this long.
const VAULT_TIMEOUT: Duration = Duration::from_secs(20);
const SERVICE_VERSION: u32 = 1;
const MAX_TOKEN_BYTES: usize = 8192;
const MAX_USERNAME_BYTES: usize = 256;

/// A blocking secret store. Implementations never expose platform detail.
pub trait CredentialVault: Send + Sync + 'static {
    fn get(&self, account: &str) -> Result<Option<String>, VaultError>;
    fn set(&self, account: &str, label: &str, secret: &str) -> Result<(), VaultError>;
    /// An absent item is success.
    fn delete(&self, account: &str) -> Result<(), VaultError>;
}

/// The vault is unreachable, locked or refused the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    Unavailable,
}

/// In-process vault for tests and test-mode hosts. `set_failing(true)` makes
/// every call fail with [`VaultError::Unavailable`].
#[derive(Default)]
pub struct MemoryVault {
    items: Mutex<BTreeMap<String, (String, String)>>,
    fail: AtomicBool,
}

impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_failing(&self, failing: bool) {
        self.fail.store(failing, Ordering::SeqCst);
    }

    /// The label stored with `account`, for asserting item metadata.
    pub fn label(&self, account: &str) -> Option<String> {
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(account)
            .map(|(label, _)| label.clone())
    }

    fn check(&self) -> Result<(), VaultError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(VaultError::Unavailable)
        } else {
            Ok(())
        }
    }
}

impl CredentialVault for MemoryVault {
    fn get(&self, account: &str) -> Result<Option<String>, VaultError> {
        self.check()?;
        Ok(self
            .items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(account)
            .map(|(_, secret)| secret.clone()))
    }

    fn set(&self, account: &str, label: &str, secret: &str) -> Result<(), VaultError> {
        self.check()?;
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(account.to_owned(), (label.to_owned(), secret.to_owned()));
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), VaultError> {
        self.check()?;
        self.items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(account);
        Ok(())
    }
}

/// The vault for hosts and targets without an OS vault: every call fails.
pub struct UnavailableVault;

impl CredentialVault for UnavailableVault {
    fn get(&self, _account: &str) -> Result<Option<String>, VaultError> {
        Err(VaultError::Unavailable)
    }
    fn set(&self, _account: &str, _label: &str, _secret: &str) -> Result<(), VaultError> {
        Err(VaultError::Unavailable)
    }
    fn delete(&self, _account: &str) -> Result<(), VaultError> {
        Err(VaultError::Unavailable)
    }
}

/// A stored credential. Not serializable; `Debug` shows the kind only.
pub struct ProviderCredential {
    kind: ProviderAuthKind,
    username: Option<String>,
    token: String,
}

impl ProviderCredential {
    pub fn kind(&self) -> ProviderAuthKind {
        self.kind
    }

    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The `Authorization` header value: `Bearer <token>` or
    /// `Basic base64(<username>:<token>)`.
    pub fn authorization(&self) -> String {
        match self.kind {
            ProviderAuthKind::Bearer => format!("Bearer {}", self.token),
            ProviderAuthKind::Basic => format!(
                "Basic {}",
                STANDARD.encode(format!(
                    "{}:{}",
                    self.username.as_deref().unwrap_or_default(),
                    self.token
                ))
            ),
        }
    }
}

impl std::fmt::Debug for ProviderCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCredential")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Which auth kinds a configured provider supports; empty means none.
pub type CredentialKinds = fn(&ProjectProvider) -> &'static [ProviderAuthKind];

/// The vault secret: UTF-8 JSON. `instance` must equal the configured one.
#[derive(Serialize, Deserialize)]
struct StoredSecret {
    version: u32,
    instance: String,
    kind: ProviderAuthKind,
    username: Option<String>,
    token: String,
}

struct Identity {
    instance: String,
    account: String,
    label: String,
}

struct Slot {
    provider_id: String,
    kinds: &'static [ProviderAuthKind],
    /// `None` when the configured `base_url` cannot name a site.
    identity: Option<Identity>,
}

impl Slot {
    fn supports(&self, kind: ProviderAuthKind) -> bool {
        self.identity.is_some() && self.kinds.contains(&kind)
    }

    fn supported_kinds(&self) -> Vec<ProviderAuthKind> {
        if self.identity.is_some() {
            self.kinds.to_vec()
        } else {
            Vec::new()
        }
    }

    fn status(
        &self,
        state: ProviderCredentialState,
        kind: Option<ProviderAuthKind>,
    ) -> ProviderCredentialStatus {
        ProviderCredentialStatus {
            provider_id: self.provider_id.clone(),
            state,
            kind,
            supported_kinds: self.supported_kinds(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VaultFailure {
    Unavailable,
    Timeout,
}

impl VaultFailure {
    fn into_error(self) -> InspectionError {
        match self {
            Self::Unavailable => InspectionError::new(
                "credential_vault_unavailable",
                "The credential vault is unavailable or locked",
            ),
            Self::Timeout => InspectionError::new(
                "credential_vault_timeout",
                "The credential vault did not respond in time",
            ),
        }
    }
}

type Cached = Option<Arc<ProviderCredential>>;

/// Credentials for the configured providers, cached per process.
///
/// The cache fills on first use; `set` and `clear` write through. A vault
/// failure is never cached, so the next call retries. Changes made to the
/// vault outside Cockpit are seen after a restart.
pub struct ProviderCredentials {
    vault: Arc<dyn CredentialVault>,
    slots: Vec<Slot>,
    cache: Mutex<BTreeMap<String, Cached>>,
    write: tokio::sync::Mutex<()>,
    vault_timeout: Duration,
}

impl ProviderCredentials {
    pub fn new(
        configuration: &ProjectConfiguration,
        vault: Arc<dyn CredentialVault>,
        kinds: CredentialKinds,
    ) -> Self {
        let slots = configuration
            .providers
            .iter()
            .map(|provider| Slot {
                provider_id: provider.id.clone(),
                kinds: kinds(provider),
                identity: identity(&provider.id, &provider.base_url),
            })
            .collect();
        Self::with_slots(slots, vault)
    }

    /// No providers and an unavailable vault, for tests and test-mode hosts.
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self::with_slots(Vec::new(), Arc::new(UnavailableVault)))
    }

    fn with_slots(slots: Vec<Slot>, vault: Arc<dyn CredentialVault>) -> Self {
        Self {
            vault,
            slots,
            cache: Mutex::new(BTreeMap::new()),
            write: tokio::sync::Mutex::new(()),
            vault_timeout: VAULT_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_vault_timeout(mut self, timeout: Duration) -> Self {
        self.vault_timeout = timeout;
        self
    }

    /// Presence and kind for every configured provider, in configuration order.
    pub async fn statuses(&self) -> ProviderCredentialStatusList {
        let mut providers = Vec::with_capacity(self.slots.len());
        let mut timed_out = false;
        for slot in &self.slots {
            let status = if !slot.supports_any() {
                slot.status(ProviderCredentialState::Unsupported, None)
            } else if timed_out {
                // A hung vault would otherwise cost one deadline per provider.
                slot.status(ProviderCredentialState::VaultUnavailable, None)
            } else {
                match self.load(slot).await {
                    Ok(Some(credential)) => {
                        slot.status(ProviderCredentialState::Stored, Some(credential.kind()))
                    }
                    Ok(None) => slot.status(ProviderCredentialState::NotStored, None),
                    Err(failure) => {
                        timed_out = failure == VaultFailure::Timeout;
                        slot.status(ProviderCredentialState::VaultUnavailable, None)
                    }
                }
            };
            providers.push(status);
        }
        ProviderCredentialStatusList { providers }
    }

    /// Validate, store and cache a credential for one provider.
    pub async fn set(
        &self,
        request: ProviderCredentialSetRequest,
    ) -> Result<ProviderCredentialStatus, InspectionError> {
        let slot = self.slot(&request.provider_id)?;
        if !slot.supports(request.kind) {
            return Err(unsupported());
        }
        validate(&request)?;
        let identity = slot.identity.as_ref().ok_or_else(unsupported)?;
        let credential = Arc::new(ProviderCredential {
            kind: request.kind,
            username: request.username,
            token: request.token,
        });
        let secret = serde_json::to_string(&StoredSecret {
            version: SERVICE_VERSION,
            instance: identity.instance.clone(),
            kind: credential.kind,
            username: credential.username.clone(),
            token: credential.token.clone(),
        })
        .map_err(|_| VaultFailure::Unavailable.into_error())?;
        let (account, label) = (identity.account.clone(), identity.label.clone());

        let _write = self.write.lock().await;
        let result = self
            .run_vault(move |vault| vault.set(&account, &label, &secret))
            .await;
        match result {
            Ok(()) => {
                self.cache_put(&slot.provider_id, Some(credential.clone()));
                Ok(slot.status(ProviderCredentialState::Stored, Some(credential.kind())))
            }
            Err(failure) => {
                // The write may or may not have landed: read the vault next time.
                self.cache_forget(&slot.provider_id);
                Err(failure.into_error())
            }
        }
    }

    /// Remove a provider's stored credential; absent is success.
    pub async fn clear(
        &self,
        request: ProviderCredentialClearRequest,
    ) -> Result<ProviderCredentialStatus, InspectionError> {
        let slot = self.slot(&request.provider_id)?;
        let identity = slot.identity.as_ref().filter(|_| slot.supports_any());
        let account = identity.ok_or_else(unsupported)?.account.clone();

        let _write = self.write.lock().await;
        match self.run_vault(move |vault| vault.delete(&account)).await {
            Ok(()) => {
                self.cache_put(&slot.provider_id, None);
                Ok(slot.status(ProviderCredentialState::NotStored, None))
            }
            Err(failure) => {
                self.cache_forget(&slot.provider_id);
                Err(failure.into_error())
            }
        }
    }

    /// The credential for native HTTP; absent or unavailable tokens are errors.
    pub async fn required(
        &self,
        provider_id: &str,
    ) -> Result<Arc<ProviderCredential>, InspectionError> {
        let slot = self.slot(provider_id)?;
        if !slot.supports_any() {
            return Err(unsupported());
        }
        match self.load(slot).await {
            Ok(Some(credential)) => Ok(credential),
            Ok(None) => Err(InspectionError::new(
                "source_credential_required",
                "No provider token is stored in Cockpit for this site",
            )),
            Err(failure) => Err(failure.into_error()),
        }
    }

    fn slot(&self, provider_id: &str) -> Result<&Slot, InspectionError> {
        self.slots
            .iter()
            .find(|slot| slot.provider_id == provider_id)
            .ok_or_else(unsupported)
    }

    async fn load(&self, slot: &Slot) -> Result<Cached, VaultFailure> {
        if let Some(cached) = self.cache_get(&slot.provider_id) {
            return Ok(cached);
        }
        let identity = slot.identity.as_ref().ok_or(VaultFailure::Unavailable)?;
        let account = identity.account.clone();
        let raw = self.run_vault(move |vault| vault.get(&account)).await?;
        let parsed = raw
            .and_then(|raw| decode(&raw, &identity.instance, slot))
            .map(Arc::new);
        // A concurrent set/clear may have written through meanwhile; keep it.
        Ok(self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(slot.provider_id.clone())
            .or_insert(parsed)
            .clone())
    }

    async fn run_vault<T: Send + 'static>(
        &self,
        call: impl FnOnce(&dyn CredentialVault) -> Result<T, VaultError> + Send + 'static,
    ) -> Result<T, VaultFailure> {
        let vault = self.vault.clone();
        let task = tokio::task::spawn_blocking(move || call(vault.as_ref()));
        match tokio::time::timeout(self.vault_timeout, task).await {
            Err(_) => Err(VaultFailure::Timeout),
            Ok(Err(_)) | Ok(Ok(Err(VaultError::Unavailable))) => Err(VaultFailure::Unavailable),
            Ok(Ok(Ok(value))) => Ok(value),
        }
    }

    fn cache_get(&self, provider_id: &str) -> Option<Cached> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider_id)
            .cloned()
    }

    fn cache_put(&self, provider_id: &str, value: Cached) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(provider_id.to_owned(), value);
    }

    fn cache_forget(&self, provider_id: &str) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(provider_id);
    }
}

impl Slot {
    /// Whether the provider can hold a credential at all.
    fn supports_any(&self) -> bool {
        self.identity.is_some() && !self.kinds.is_empty()
    }
}

fn unsupported() -> InspectionError {
    InspectionError::new(
        "credential_provider_unsupported",
        "This provider does not support a stored credential",
    )
}

fn invalid(message: &'static str) -> InspectionError {
    InspectionError::new("invalid_credential_request", message)
}

fn validate(request: &ProviderCredentialSetRequest) -> Result<(), InspectionError> {
    let token = &request.token;
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        return Err(invalid("The token must be between 1 and 8192 bytes"));
    }
    if token.chars().any(char::is_control) {
        return Err(invalid("The token must not contain control characters"));
    }
    if token.trim().is_empty() {
        return Err(invalid("The token must not be blank"));
    }
    match (request.kind, request.username.as_deref()) {
        (ProviderAuthKind::Bearer, None) => Ok(()),
        (ProviderAuthKind::Bearer, Some(_)) => {
            Err(invalid("A bearer token does not take a username"))
        }
        (ProviderAuthKind::Basic, None) => Err(invalid("Basic credentials require a username")),
        (ProviderAuthKind::Basic, Some(username)) => {
            if username.is_empty() || username.len() > MAX_USERNAME_BYTES {
                Err(invalid("The username must be between 1 and 256 bytes"))
            } else if username.chars().any(char::is_control) || username.contains(':') {
                Err(invalid(
                    "The username must not contain control characters or ':'",
                ))
            } else {
                Ok(())
            }
        }
    }
}

/// `None` when the item is malformed, from another site, or no longer a
/// supported kind: it counts as not stored.
fn decode(raw: &str, instance: &str, slot: &Slot) -> Option<ProviderCredential> {
    let secret: StoredSecret = serde_json::from_str(raw).ok()?;
    let usable = secret.version == SERVICE_VERSION
        && secret.instance == instance
        && slot.supports(secret.kind)
        && !secret.token.is_empty()
        && match secret.kind {
            ProviderAuthKind::Bearer => secret.username.is_none(),
            ProviderAuthKind::Basic => secret.username.as_deref().is_some_and(|u| !u.is_empty()),
        };
    usable.then_some(ProviderCredential {
        kind: secret.kind,
        username: secret.username,
        token: secret.token,
    })
}

fn identity(provider_id: &str, base_url: &str) -> Option<Identity> {
    let instance = instance(base_url)?;
    let host = url::Url::parse(base_url)
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    Some(Identity {
        account: format!("{provider_id} {instance}"),
        label: format!("Cockpit {provider_id} token ({host})"),
        instance,
    })
}

/// `scheme://host[:port]<path without trailing />`, host lowercased, default
/// port dropped.
fn instance(base_url: &str) -> Option<String> {
    let url = url::Url::parse(base_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!(
        "{}://{host}{port}{}",
        url.scheme(),
        url.path().trim_end_matches('/')
    ))
}

#[cfg(test)]
mod tests {

    use cockpit_protocol::projects::{ProjectLimits, ProviderKind, ProviderDeployment};

    use super::*;

    const BOTH: &[ProviderAuthKind] = &[ProviderAuthKind::Bearer, ProviderAuthKind::Basic];
    const BEARER: &[ProviderAuthKind] = &[ProviderAuthKind::Bearer];
    const TOKEN: &str = "s3cr3t-token-value";

    fn kinds(provider: &ProjectProvider) -> &'static [ProviderAuthKind] {
        match provider.kind {
            ProviderKind::Jira => BOTH,
            ProviderKind::Confluence => BEARER,
            _ => &[],
        }
    }

    fn configuration(providers: &[(&str, &str)]) -> ProjectConfiguration {
        ProjectConfiguration {
        repository_roots: vec![],
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: providers
            .iter()
            .map(|(id, base_url)| ProjectProvider {
                id: (*id).into(),
                kind: match *id {
                    "jira" => ProviderKind::Jira,
                    "confluence" => ProviderKind::Confluence,
                    _ => ProviderKind::Gitlab,
                },
                base_url: (*base_url).into(),
                executable: (*id == "gitlab").then(|| "glab".into()),
                login: None,
                deployment: (*id != "gitlab").then_some(ProviderDeployment::Cloud),
            })
            .collect(),
        limits: ProjectLimits {
            catalog_depth: 1,
            catalog_entries: 1,
            git_timeout_ms: 1000,
            git_output_bytes: 65536,
            operation_timeout_ms: 1000,
            context_preview_bytes: 1024,
            context_preview_lines: 100,
            context_directory_entries: 1,
            context_tree_depth: 1,
            library_folder_files: 512,
            library_folder_bytes: 32 * 1024 * 1024,
            library_file_bytes: 4 * 1024 * 1024,
            library_space_pages: 200,
            library_attachment_bytes: 25 * 1024 * 1024,
            library_item_attachment_bytes: 100 * 1024 * 1024,
            library_max_items: 20_000,
        },
        ..ProjectConfiguration::for_tests(std::path::Path::new(""))
        }
    }

    fn service(vault: &Arc<MemoryVault>, jira_url: &str) -> ProviderCredentials {
        ProviderCredentials::new(
            &configuration(&[
                ("jira", jira_url),
                ("confluence", "https://example.atlassian.net/wiki"),
                ("gitlab", "https://gitlab.com"),
            ]),
            vault.clone(),
            kinds,
        )
    }

    fn set_request(
        provider_id: &str,
        kind: ProviderAuthKind,
        username: Option<&str>,
        token: &str,
    ) -> ProviderCredentialSetRequest {
        ProviderCredentialSetRequest {
            provider_id: provider_id.into(),
            kind,
            username: username.map(str::to_owned),
            token: token.into(),
        }
    }

    fn bearer(provider_id: &str) -> ProviderCredentialSetRequest {
        set_request(provider_id, ProviderAuthKind::Bearer, None, TOKEN)
    }

    fn clear_request(provider_id: &str) -> ProviderCredentialClearRequest {
        ProviderCredentialClearRequest {
            provider_id: provider_id.into(),
        }
    }

    async fn state_of(
        credentials: &ProviderCredentials,
        provider_id: &str,
    ) -> ProviderCredentialStatus {
        credentials
            .statuses()
            .await
            .providers
            .into_iter()
            .find(|status| status.provider_id == provider_id)
            .expect("provider status")
    }

    #[test]
    fn instance_is_normalised_per_site() {
        assert_eq!(
            instance("HTTPS://Example.COM:443/wiki/").as_deref(),
            Some("https://example.com/wiki")
        );
        assert_eq!(
            instance("http://Jira.Local:8080").as_deref(),
            Some("http://jira.local:8080")
        );
        assert_eq!(
            instance("https://a.example/x/y//").as_deref(),
            Some("https://a.example/x/y")
        );
        assert_eq!(instance("ftp://example.com"), None);
        assert_eq!(instance("not a url"), None);
        let identity = identity("jira", "https://Team.Atlassian.net/").expect("identity");
        assert_eq!(identity.account, "jira https://team.atlassian.net");
        assert_eq!(identity.label, "Cockpit jira token (team.atlassian.net)");
    }

    #[tokio::test]
    async fn set_reports_stored_kind_and_clear_reports_not_stored() {
        let vault = Arc::new(MemoryVault::new());
        let credentials = service(&vault, "https://team.atlassian.net");
        assert_eq!(
            state_of(&credentials, "jira").await.state,
            ProviderCredentialState::NotStored
        );
        let status = credentials
            .set(set_request(
                "jira",
                ProviderAuthKind::Basic,
                Some("me@example.com"),
                TOKEN,
            ))
            .await
            .expect("set");
        assert_eq!(status.state, ProviderCredentialState::Stored);
        assert_eq!(status.kind, Some(ProviderAuthKind::Basic));
        assert_eq!(status.supported_kinds, BOTH);
        let listed = state_of(&credentials, "jira").await;
        assert_eq!(
            (listed.state, listed.kind),
            (
                ProviderCredentialState::Stored,
                Some(ProviderAuthKind::Basic)
            )
        );
        assert_eq!(
            vault.label("jira https://team.atlassian.net").as_deref(),
            Some("Cockpit jira token (team.atlassian.net)")
        );

        let status = credentials
            .clear(clear_request("jira"))
            .await
            .expect("clear");
        assert_eq!(
            (status.state, status.kind),
            (ProviderCredentialState::NotStored, None)
        );
        assert_eq!(credentials.required("jira").await.unwrap_err().code, "source_credential_required");
        // Clearing an absent item is success.
        credentials
            .clear(clear_request("jira"))
            .await
            .expect("clear again");
    }

    #[tokio::test]
    async fn invalid_requests_are_rejected_and_nothing_is_stored() {
        let vault = Arc::new(MemoryVault::new());
        let credentials = service(&vault, "https://team.atlassian.net");
        let bad = [
            set_request("jira", ProviderAuthKind::Basic, None, TOKEN),
            set_request("jira", ProviderAuthKind::Basic, Some(""), TOKEN),
            set_request("jira", ProviderAuthKind::Basic, Some("a:b"), TOKEN),
            set_request("jira", ProviderAuthKind::Basic, Some("a\nb"), TOKEN),
            set_request(
                "jira",
                ProviderAuthKind::Basic,
                Some(&"u".repeat(257)),
                TOKEN,
            ),
            set_request("jira", ProviderAuthKind::Bearer, Some("me"), TOKEN),
            set_request("jira", ProviderAuthKind::Bearer, None, "line\nbreak"),
            set_request("jira", ProviderAuthKind::Bearer, None, "nul\0byte"),
            set_request("jira", ProviderAuthKind::Bearer, None, ""),
            set_request("jira", ProviderAuthKind::Bearer, None, " \t "),
            set_request("jira", ProviderAuthKind::Bearer, None, &"t".repeat(8193)),
        ];
        for request in bad {
            let error = credentials.set(request).await.expect_err("rejected");
            assert_eq!(error.code, "invalid_credential_request", "{error}");
        }
        assert_eq!(
            state_of(&credentials, "jira").await.state,
            ProviderCredentialState::NotStored
        );
        assert_eq!(vault.get("jira https://team.atlassian.net"), Ok(None));
        credentials
            .set(set_request(
                "jira",
                ProviderAuthKind::Bearer,
                None,
                &"t".repeat(8192),
            ))
            .await
            .expect("boundary length accepted");
    }

    #[tokio::test]
    async fn unsupported_kind_and_provider_are_rejected() {
        let vault = Arc::new(MemoryVault::new());
        let credentials = service(&vault, "https://team.atlassian.net");
        for request in [
            set_request("confluence", ProviderAuthKind::Basic, Some("u"), TOKEN),
            bearer("gitlab"),
            bearer("unknown"),
        ] {
            let error = credentials.set(request).await.expect_err("unsupported");
            assert_eq!(error.code, "credential_provider_unsupported");
        }
        for id in ["gitlab", "unknown"] {
            let error = credentials
                .clear(clear_request(id))
                .await
                .expect_err("unsupported");
            assert_eq!(error.code, "credential_provider_unsupported");
        }
        let gitlab = state_of(&credentials, "gitlab").await;
        assert_eq!(gitlab.state, ProviderCredentialState::Unsupported);
        assert!(gitlab.supported_kinds.is_empty());
        // No vault access for providers that cannot hold a credential.
        vault.set_failing(true);
        assert_eq!(
            state_of(&credentials, "gitlab").await.state,
            ProviderCredentialState::Unsupported
        );
        assert_eq!(
            credentials.required("gitlab").await.unwrap_err().code,
            "credential_provider_unsupported"
        );
    }

    #[tokio::test]
    async fn changing_base_url_makes_the_prior_secret_not_stored() {
        let vault = Arc::new(MemoryVault::new());
        let old = service(&vault, "https://old.atlassian.net");
        old.set(bearer("jira")).await.expect("set");
        assert!(old.required("jira").await.is_ok());

        let moved = service(&vault, "https://new.atlassian.net");
        assert_eq!(
            state_of(&moved, "jira").await.state,
            ProviderCredentialState::NotStored
        );
        assert_eq!(moved.required("jira").await.unwrap_err().code, "source_credential_required");
        // The old site's item is untouched and still usable for that site.
        assert!(
            service(&vault, "https://old.atlassian.net/")
                .required("jira")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn secret_from_another_instance_is_not_stored() {
        let vault = Arc::new(MemoryVault::new());
        let credentials = service(&vault, "https://team.atlassian.net");
        let account = "jira https://team.atlassian.net";
        let secret = |instance: &str, version: u32| {
            format!(
                r#"{{"version":{version},"instance":"{instance}","kind":"bearer","username":null,"token":"{TOKEN}"}}"#
            )
        };
        // The account matches but the recorded instance does not.
        vault
            .set(account, "l", &secret("https://other.atlassian.net", 1))
            .unwrap();
        assert_eq!(credentials.required("jira").await.unwrap_err().code, "source_credential_required");
        assert_eq!(
            state_of(&credentials, "jira").await.state,
            ProviderCredentialState::NotStored
        );

        for raw in [
            secret("https://team.atlassian.net", 2),
            "not json".to_owned(),
            r#"{"version":1,"instance":"https://team.atlassian.net","kind":"basic","username":null,"token":"t"}"#.to_owned(),
        ] {
            let fresh = service(&vault, "https://team.atlassian.net");
            vault.set(account, "l", &raw).unwrap();
            assert_eq!(fresh.required("jira").await.unwrap_err().code, "source_credential_required");
        }

        vault
            .set(account, "l", &secret("https://team.atlassian.net", 1))
            .unwrap();
        let fresh = service(&vault, "https://team.atlassian.net");
        assert_eq!(fresh.required("jira").await.expect("stored").token(), TOKEN);
    }

    #[tokio::test]
    async fn vault_failure_is_reported_and_not_cached() {
        let vault = Arc::new(MemoryVault::new());
        let seed = service(&vault, "https://team.atlassian.net");
        seed.set(bearer("jira")).await.expect("seed");

        let credentials = service(&vault, "https://team.atlassian.net");
        vault.set_failing(true);
        assert_eq!(
            state_of(&credentials, "jira").await.state,
            ProviderCredentialState::VaultUnavailable
        );
        assert_eq!(
            credentials.required("jira").await.unwrap_err().code,
            "credential_vault_unavailable"
        );

        vault.set_failing(false);
        assert_eq!(
            credentials
                .required("jira")
                .await
                .expect("recovered")
                .token(),
            TOKEN
        );
    }

    #[tokio::test]
    async fn required_distinguishes_missing_from_unavailable() {
        let vault = Arc::new(MemoryVault::new());
        let credentials = service(&vault, "https://team.atlassian.net");
        let error = credentials.required("jira").await.unwrap_err();
        assert_eq!(error.code, "source_credential_required");
        assert_eq!(
            credentials.required("nope").await.unwrap_err().code,
            "credential_provider_unsupported"
        );

        credentials.set(bearer("jira")).await.expect("set");
        assert_eq!(
            credentials.required("jira").await.expect("stored").kind(),
            ProviderAuthKind::Bearer
        );

        let disabled = ProviderCredentials::disabled();
        assert_eq!(
            disabled.required("jira").await.unwrap_err().code,
            "credential_provider_unsupported"
        );
        assert!(disabled.statuses().await.providers.is_empty());

        let unavailable = ProviderCredentials::new(
            &configuration(&[("jira", "https://team.atlassian.net")]),
            Arc::new(UnavailableVault),
            kinds,
        );
        assert_eq!(
            unavailable.required("jira").await.unwrap_err().code,
            "credential_vault_unavailable"
        );
        assert_eq!(
            unavailable.set(bearer("jira")).await.unwrap_err().code,
            "credential_vault_unavailable"
        );
        assert_eq!(
            unavailable
                .clear(clear_request("jira"))
                .await
                .unwrap_err()
                .code,
            "credential_vault_unavailable"
        );
    }

    #[tokio::test]
    async fn set_and_clear_write_through_the_cache() {
        let vault = Arc::new(MemoryVault::new());
        let credentials = service(&vault, "https://team.atlassian.net");
        credentials.set(bearer("jira")).await.expect("set");
        // Served from the cache: no vault access needed.
        vault.set_failing(true);
        assert_eq!(
            credentials.required("jira").await.expect("cached").token(),
            TOKEN
        );
        assert_eq!(
            state_of(&credentials, "jira").await.state,
            ProviderCredentialState::Stored
        );

        // A failed replace does not pretend to have stored anything, and the
        // next call consults the vault.
        let error = credentials
            .set(set_request(
                "jira",
                ProviderAuthKind::Bearer,
                None,
                "replacement",
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, "credential_vault_unavailable");
        assert_eq!(credentials.required("jira").await.unwrap_err().code, "credential_vault_unavailable");
        vault.set_failing(false);
        assert_eq!(
            credentials.required("jira").await.expect("original").token(),
            TOKEN
        );

        credentials
            .set(set_request(
                "jira",
                ProviderAuthKind::Bearer,
                None,
                "replacement",
            ))
            .await
            .expect("replace");
        assert_eq!(
            credentials.required("jira").await.expect("replaced").token(),
            "replacement"
        );

        credentials
            .clear(clear_request("jira"))
            .await
            .expect("clear");
        vault.set_failing(true);
        assert_eq!(
            credentials.required("jira").await.unwrap_err().code,
            "source_credential_required",
            "a cleared credential is served from the cache"
        );
    }

    struct SlowVault;

    impl CredentialVault for SlowVault {
        fn get(&self, _account: &str) -> Result<Option<String>, VaultError> {
            std::thread::sleep(Duration::from_millis(400));
            Ok(None)
        }
        fn set(&self, _account: &str, _label: &str, _secret: &str) -> Result<(), VaultError> {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        }
        fn delete(&self, _account: &str) -> Result<(), VaultError> {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_slow_vault_maps_to_timeout_and_is_not_cached() {
        let credentials = ProviderCredentials::new(
            &configuration(&[
                ("jira", "https://team.atlassian.net"),
                ("confluence", "https://example.atlassian.net/wiki"),
            ]),
            Arc::new(SlowVault),
            kinds,
        )
        .with_vault_timeout(Duration::from_millis(50));

        assert_eq!(
            credentials.required("jira").await.unwrap_err().code,
            "credential_vault_timeout"
        );
        assert_eq!(
            credentials.set(bearer("jira")).await.unwrap_err().code,
            "credential_vault_timeout"
        );
        assert_eq!(
            credentials
                .clear(clear_request("jira"))
                .await
                .unwrap_err()
                .code,
            "credential_vault_timeout"
        );
        // Nothing was cached by the failures: the vault is asked again.
        assert!(credentials.cache_get("jira").is_none());

        // One hung vault costs one deadline for the whole list, not one each.
        let started = std::time::Instant::now();
        let list = credentials.statuses().await;
        assert!(
            list.providers
                .iter()
                .all(|s| s.state == ProviderCredentialState::VaultUnavailable)
        );
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let credential = ProviderCredential {
            kind: ProviderAuthKind::Basic,
            username: Some("someone@example.com".into()),
            token: TOKEN.into(),
        };
        let shown = format!("{credential:?} {credential:#?}");
        assert!(shown.contains("Basic"));
        assert!(
            !shown.contains(TOKEN) && !shown.contains("someone"),
            "{shown}"
        );

        let request = set_request(
            "jira",
            ProviderAuthKind::Basic,
            Some("someone@example.com"),
            TOKEN,
        );
        let shown = format!("{request:?} {request:#?}");
        assert!(shown.contains("jira") && shown.contains("Basic"));
        assert!(
            !shown.contains(TOKEN) && !shown.contains("someone"),
            "{shown}"
        );
    }

    #[test]
    fn authorization_header_matches_the_kind() {
        let bearer = ProviderCredential {
            kind: ProviderAuthKind::Bearer,
            username: None,
            token: "abc".into(),
        };
        assert_eq!(bearer.authorization(), "Bearer abc");
        let basic = ProviderCredential {
            kind: ProviderAuthKind::Basic,
            username: Some("me@x.io".into()),
            token: "abc".into(),
        };
        assert_eq!(
            basic.authorization(),
            format!("Basic {}", STANDARD.encode("me@x.io:abc"))
        );
    }
}
