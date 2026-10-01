use super::{ProviderRecord, RETAIN, empty};
use cockpit_protocol::quota::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Omp {
    reports: Vec<Report>,
    #[serde(default)]
    accounts_without_usage: Vec<WithoutUsage>,
}
#[derive(Deserialize)]
struct WithoutUsage {
    provider: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    provider: String,
    fetched_at: u64,
    limits: Vec<Limit>,
}
#[derive(Deserialize)]
struct Limit {
    id: Option<String>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    scope: Scope,
    window: Option<Window>,
    amount: Amount,
    status: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scope {
    tier: Option<String>,
    model_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    id: Option<String>,
    duration_ms: Option<u64>,
    resets_at: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Amount {
    unit: String,
    used: Option<f64>,
    limit: Option<f64>,
    remaining: Option<f64>,
    used_fraction: Option<f64>,
    remaining_fraction: Option<f64>,
}

pub(super) fn omp(bytes: &[u8], now: u64) -> Result<Vec<ProviderRecord>, QuotaErrorCode> {
    let input: Omp = serde_json::from_slice(bytes).map_err(|_| QuotaErrorCode::Malformed)?;
    let mut output = Vec::with_capacity(3);
    for (provider, source) in [
        (QuotaProvider::Codex, "openai-codex"),
        (QuotaProvider::Claude, "anthropic"),
        (QuotaProvider::Copilot, "github-copilot"),
    ] {
        let mut accounts = Vec::new();
        let mut reported = false;
        let mut retained_report = false;
        for report in input
            .reports
            .iter()
            .filter(|report| report.provider == source)
            .take(8)
        {
            reported = true;
            if report.fetched_at > now.saturating_add(60_000) {
                return Err(QuotaErrorCode::Malformed);
            }
            if now.saturating_sub(report.fetched_at) > RETAIN {
                continue;
            }
            retained_report = true;
            let mut limits = Vec::new();
            for input in &report.limits {
                let credits = provider == QuotaProvider::Copilot;
                let supported = if credits {
                    input.id.as_deref() == Some("copilot:premium")
                        && matches!(input.amount.unit.as_str(), "requests" | "credits")
                } else {
                    input.amount.unit == "percent"
                };
                if !supported {
                    continue;
                }
                if limits.len() == 24 {
                    break;
                }
                let amount = &input.amount;
                if credits
                    && [amount.used, amount.limit, amount.remaining]
                        .into_iter()
                        .any(|value| value.is_some_and(|value| !number(value, 1e9)))
                {
                    return Err(QuotaErrorCode::Malformed);
                }
                let unlimited = credits
                    && amount.used.is_none()
                    && amount.limit.is_none()
                    && amount.remaining.is_none()
                    && input.notes.iter().any(|note| note == "Unlimited");
                let fraction = if unlimited {
                    None
                } else {
                    amount
                        .used_fraction
                        .or_else(|| {
                            amount
                                .used
                                .zip(amount.limit)
                                .filter(|(_, total)| *total > 0.0)
                                .map(|(used, total)| used / total)
                        })
                        .or_else(|| {
                            if credits {
                                None
                            } else {
                                amount.used.map(|used| used / 100.0)
                            }
                        })
                        .or_else(|| amount.remaining_fraction.map(|remaining| 1.0 - remaining))
                };
                if fraction.is_some_and(|value| !number(value, 10.0)) {
                    return Err(QuotaErrorCode::Malformed);
                }
                let window = input
                    .window
                    .as_ref()
                    .and_then(|window| normalize_window(window.id.as_deref(), window.duration_ms));
                let tier =
                    if credits {
                        None
                    } else {
                        input
                            .scope
                            .tier
                            .as_deref()
                            .filter(|tier| known_tier(tier))
                            .or_else(|| {
                                input.scope.model_id.as_deref().and_then(|model| {
                                    model.split('-').find(|part| known_tier(part))
                                })
                            })
                            .map(str::to_owned)
                    };
                let level = match input.status.as_deref() {
                    Some("ok") => QuotaLevel::Ok,
                    Some("warning") => QuotaLevel::Warning,
                    Some("exhausted") => QuotaLevel::Exhausted,
                    Some("unknown") => QuotaLevel::Unknown,
                    _ => level(fraction),
                };
                limits.push(QuotaLimit {
                    id: id(provider, window.as_deref(), limits.len()),
                    window,
                    tier,
                    // OMP Copilot counters are surfaced as credits without rescaling,
                    // including reports carrying legacy source unit labels.
                    unit: if credits {
                        QuotaUnit::Credits
                    } else {
                        QuotaUnit::Percent
                    },
                    used_fraction: fraction,
                    used: credits.then_some(amount.used).flatten(),
                    limit: credits.then_some(amount.limit).flatten(),
                    remaining: credits.then_some(amount.remaining).flatten(),
                    unlimited,
                    level,
                    resets_at_ms: input
                        .window
                        .as_ref()
                        .and_then(|window| window.resets_at)
                        .filter(|time| *time <= 9_007_199_254_740_991),
                });
            }
            if !limits.is_empty() {
                accounts.push(QuotaAccount {
                    fetched_at_ms: report.fetched_at,
                    limits,
                });
            }
        }
        output.push(if !accounts.is_empty() {
            ProviderRecord {
                provider,
                state: QuotaProviderState::Available,
                error: None,
                accounts,
            }
        } else if retained_report {
            empty(
                provider,
                QuotaProviderState::Unsupported,
                Some(QuotaErrorCode::Unsupported),
            )
        } else if input
            .accounts_without_usage
            .iter()
            .any(|account| account.provider == source)
        {
            empty(
                provider,
                QuotaProviderState::Unavailable,
                Some(QuotaErrorCode::UsageUnavailable),
            )
        } else if reported {
            empty(
                provider,
                QuotaProviderState::Unavailable,
                Some(QuotaErrorCode::UsageUnavailable),
            )
        } else {
            empty(
                provider,
                QuotaProviderState::NotSignedIn,
                Some(QuotaErrorCode::NotSignedIn),
            )
        });
    }
    Ok(output)
}

fn number(value: f64, max: f64) -> bool {
    value.is_finite() && (0.0..=max).contains(&value)
}
fn level(fraction: Option<f64>) -> QuotaLevel {
    match fraction {
        Some(value) if value >= 1.0 => QuotaLevel::Exhausted,
        Some(value) if value >= 0.8 => QuotaLevel::Warning,
        Some(_) => QuotaLevel::Ok,
        None => QuotaLevel::Unknown,
    }
}
fn known_tier(tier: &str) -> bool {
    matches!(tier, "opus" | "sonnet" | "haiku" | "fable")
}
fn valid_window(window: &str) -> bool {
    if matches!(window, "monthly" | "weekly") {
        return true;
    }
    let bytes = window.as_bytes();
    (2..=5).contains(&bytes.len())
        && matches!(bytes[bytes.len() - 1], b's' | b'm' | b'h' | b'd' | b'w')
        && bytes[..bytes.len() - 1].iter().all(u8::is_ascii_digit)
}
fn normalize_window(window: Option<&str>, duration: Option<u64>) -> Option<String> {
    if let Some(window) = window.filter(|window| valid_window(window)) {
        return Some(window.into());
    }
    let duration = duration.filter(|duration| *duration > 0)?;
    for (unit, millis) in [
        ("w", 604_800_000),
        ("d", 86_400_000),
        ("h", 3_600_000),
        ("m", 60_000),
        ("s", 1_000),
    ] {
        if duration % millis == 0 && duration / millis <= 9999 {
            return Some(format!("{}{unit}", duration / millis));
        }
    }
    None
}
fn id(provider: QuotaProvider, window: Option<&str>, index: usize) -> String {
    let provider = match provider {
        QuotaProvider::Codex => "codex",
        QuotaProvider::Claude => "claude",
        QuotaProvider::Copilot => "copilot",
    };
    format!("{provider}:{}:{index}", window.unwrap_or("none"))
}
pub(super) fn valid_limit(limit: &QuotaLimit, provider: QuotaProvider, index: usize) -> bool {
    let credits = provider == QuotaProvider::Copilot;
    limit.id == id(provider, limit.window.as_deref(), index)
        && limit.window.as_deref().is_none_or(valid_window)
        && limit.tier.as_deref().is_none_or(known_tier)
        && (limit.unit
            == if credits {
                QuotaUnit::Credits
            } else {
                QuotaUnit::Percent
            })
        && limit.used_fraction.is_none_or(|value| number(value, 10.0))
        && [limit.used, limit.limit, limit.remaining]
            .into_iter()
            .all(|value| value.is_none_or(|value| credits && number(value, 1e9)))
        && (!limit.unlimited
            || (credits
                && limit.used_fraction.is_none()
                && limit.used.is_none()
                && limit.limit.is_none()
                && limit.remaining.is_none()))
        && limit
            .resets_at_ms
            .is_none_or(|time| time <= 9_007_199_254_740_991)
}
