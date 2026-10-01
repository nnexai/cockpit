//! Identity-free subscription quotas. No CLI text, account identity, or credentials.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuotaProvider {
    Codex,
    Claude,
    Copilot,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuotaProviderState {
    Pending,
    Available,
    NotSignedIn,
    Unsupported,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuotaErrorCode {
    SourceMissing,
    NotSignedIn,
    UsageUnavailable,
    Unsupported,
    Failed,
    Timeout,
    Malformed,
    CacheUnavailable,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    Percent,
    Credits,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuotaLevel {
    Ok,
    Warning,
    Exhausted,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct QuotaLimit {
    pub id: String,
    pub window: Option<String>,
    pub tier: Option<String>,
    pub unit: QuotaUnit,
    pub used_fraction: Option<f64>,
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub unlimited: bool,
    pub level: QuotaLevel,
    #[ts(type = "number | null")]
    pub resets_at_ms: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct QuotaAccount {
    /// Source-reported fetch time from the corresponding OMP report.
    #[ts(type = "number")]
    pub fetched_at_ms: u64,
    pub limits: Vec<QuotaLimit>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct QuotaProviderStatus {
    pub provider: QuotaProvider,
    pub state: QuotaProviderState,
    pub error: Option<QuotaErrorCode>,
    #[ts(type = "number | null")]
    pub fetched_at_ms: Option<u64>,
    pub stale: bool,
    pub accounts: Vec<QuotaAccount>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct QuotaStatusResponse {
    #[ts(type = "number")]
    pub generated_at_ms: u64,
    pub collecting: bool,
    pub providers: Vec<QuotaProviderStatus>,
}
