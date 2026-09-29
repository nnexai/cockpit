//! Provider credential DTOs.
//!
//! The credential surface is write-only: a token can be set, replaced and
//! removed, and its presence and kind can be listed, but no response type ever
//! carries a token or a username.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthKind {
    Bearer,
    Basic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCredentialState {
    Stored,
    NotStored,
    VaultUnavailable,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProviderCredentialStatus {
    pub provider_id: String,
    pub state: ProviderCredentialState,
    /// `Some` only when `state` is `Stored`.
    pub kind: Option<ProviderAuthKind>,
    /// Empty means the provider does not support a stored credential.
    pub supported_kinds: Vec<ProviderAuthKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProviderCredentialStatusList {
    pub providers: Vec<ProviderCredentialStatus>,
}

/// Write-only request: `Deserialize` and `TS` only, never `Serialize`.
/// `Debug` prints the provider id and kind only.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProviderCredentialSetRequest {
    pub provider_id: String,
    pub kind: ProviderAuthKind,
    /// Required for `basic`; 1..=256 bytes, no control characters, no `:`.
    #[serde(default)]
    #[ts(optional)]
    pub username: Option<String>,
    /// 1..=8192 bytes, no control characters, not all whitespace.
    pub token: String,
}

impl std::fmt::Debug for ProviderCredentialSetRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCredentialSetRequest")
            .field("provider_id", &self.provider_id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProviderCredentialClearRequest {
    pub provider_id: String,
}
