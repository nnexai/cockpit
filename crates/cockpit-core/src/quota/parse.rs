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
    used_fraction: Option<f64>,
    remaining_fraction: Option<f64>,
}

pub(super) fn omp(bytes: &[u8], now: u64) -> Result<Vec<ProviderRecord>, QuotaErrorCode> {
    let input: Omp = serde_json::from_slice(bytes).map_err(|_| QuotaErrorCode::Malformed)?;
    let mut output = Vec::with_capacity(2);
    for (provider, source) in [
        (QuotaProvider::Codex, "openai-codex"),
        (QuotaProvider::Claude, "anthropic"),
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
                if input.amount.unit != "percent" {
                    continue;
                }
                if limits.len() == 24 {
                    break;
                }
                let amount = &input.amount;
                let fraction = amount
                    .used_fraction
                    .or_else(|| {
                        amount
                            .used
                            .zip(amount.limit)
                            .filter(|(_, total)| *total > 0.0)
                            .map(|(used, total)| used / total)
                    })
                    .or_else(|| amount.used.map(|used| used / 100.0))
                    .or_else(|| amount.remaining_fraction.map(|remaining| 1.0 - remaining));
                if fraction.is_some_and(|value| !number(value, 10.0)) {
                    return Err(QuotaErrorCode::Malformed);
                }
                let window = input
                    .window
                    .as_ref()
                    .and_then(|window| normalize_window(window.id.as_deref(), window.duration_ms));
                let tier = input
                    .scope
                    .tier
                    .as_deref()
                    .filter(|tier| known_tier(tier))
                    .or_else(|| {
                        input
                            .scope
                            .model_id
                            .as_deref()
                            .and_then(|model| model.split('-').find(|part| known_tier(part)))
                    })
                    .map(str::to_owned);
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
                    unit: QuotaUnit::Percent,
                    used_fraction: fraction,
                    used: None,
                    limit: None,
                    remaining: None,
                    unlimited: false,
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

#[derive(Deserialize)]
struct Copilot {
    #[serde(default, deserialize_with = "optional")]
    token_based_billing: Option<bool>,
    #[serde(default, deserialize_with = "optional")]
    quota_snapshots: Option<Snapshots>,
    #[serde(default, deserialize_with = "optional")]
    quota_reset_date_utc: Option<String>,
}
#[derive(Deserialize)]
struct Snapshots {
    #[serde(default, deserialize_with = "optional")]
    premium_interactions: Option<Credits>,
}
#[derive(Deserialize)]
struct Credits {
    #[serde(default, deserialize_with = "optional")]
    token_based_billing: Option<bool>,
    #[serde(default, deserialize_with = "optional")]
    credits_used: Option<f64>,
    #[serde(default, deserialize_with = "optional")]
    remaining: Option<f64>,
    #[serde(default, deserialize_with = "optional")]
    entitlement: Option<f64>,
    #[serde(default, deserialize_with = "optional")]
    unlimited: Option<bool>,
    #[serde(default, deserialize_with = "optional")]
    timestamp_utc: Option<String>,
}
fn optional<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

pub(super) fn copilot(bytes: &[u8], now: u64) -> Result<Vec<ProviderRecord>, QuotaErrorCode> {
    let input: Copilot = serde_json::from_slice(bytes).map_err(|_| QuotaErrorCode::Malformed)?;
    let unsupported = || {
        Ok(vec![empty(
            QuotaProvider::Copilot,
            QuotaProviderState::Unsupported,
            Some(QuotaErrorCode::Unsupported),
        )])
    };
    if input.token_based_billing != Some(true) {
        return unsupported();
    }
    let Some(credits) = input
        .quota_snapshots
        .and_then(|snapshots| snapshots.premium_interactions)
    else {
        return unsupported();
    };
    let (Some(used), Some(remaining), Some(total), Some(unlimited)) = (
        credits.credits_used,
        credits.remaining,
        credits.entitlement,
        credits.unlimited,
    ) else {
        return unsupported();
    };
    if credits.token_based_billing != Some(true)
        || ![used, remaining, total]
            .into_iter()
            .all(|value| number(value, 1e9))
    {
        return unsupported();
    }
    let fetched_at_ms = credits
        .timestamp_utc
        .as_deref()
        .and_then(timestamp)
        .unwrap_or(now);
    if fetched_at_ms > now.saturating_add(60_000) {
        return Err(QuotaErrorCode::Malformed);
    }
    if now.saturating_sub(fetched_at_ms) > RETAIN {
        return Ok(vec![empty(
            QuotaProvider::Copilot,
            QuotaProviderState::Unavailable,
            Some(QuotaErrorCode::UsageUnavailable),
        )]);
    }
    let fraction = if unlimited || total == 0.0 {
        None
    } else {
        Some(used / total)
    };
    if fraction.is_some_and(|fraction| !number(fraction, 10.0)) {
        return Err(QuotaErrorCode::Malformed);
    }
    let limit = QuotaLimit {
        id: id(QuotaProvider::Copilot, Some("monthly"), 0),
        window: Some("monthly".into()),
        tier: None,
        unit: QuotaUnit::Credits,
        used_fraction: fraction,
        used: (!unlimited).then_some(used),
        limit: (!unlimited).then_some(total),
        remaining: (!unlimited).then_some(remaining),
        unlimited,
        level: level(fraction),
        resets_at_ms: input.quota_reset_date_utc.as_deref().and_then(timestamp),
    };
    Ok(vec![ProviderRecord {
        provider: QuotaProvider::Copilot,
        state: QuotaProviderState::Available,
        error: None,
        accounts: vec![QuotaAccount {
            fetched_at_ms,
            limits: vec![limit],
        }],
    }])
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

/// Strict UTC dates only, without timezone/local-calendar guesses.
fn timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let parse = |start, end| text.get(start..end)?.parse::<i64>().ok();
    let year = parse(0, 4)?;
    let month = parse(5, 7)?;
    let day = parse(8, 10)?;
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days).contains(&day) {
        return None;
    }
    let (hour, minute, second, millis) = if bytes.len() == 10 {
        (0, 0, 0, 0)
    } else {
        if bytes.len() < 20
            || bytes[10] != b'T'
            || bytes[13] != b':'
            || bytes[16] != b':'
            || *bytes.last()? != b'Z'
        {
            return None;
        }
        let hour = parse(11, 13)?;
        let minute = parse(14, 16)?;
        let second = parse(17, 19)?;
        if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=59).contains(&second) {
            return None;
        }
        let millis = if bytes.len() == 20 {
            0
        } else {
            if bytes[19] != b'.' {
                return None;
            }
            let fraction = &bytes[20..bytes.len() - 1];
            if fraction.is_empty() || !fraction.iter().all(u8::is_ascii_digit) {
                return None;
            }
            fraction
                .iter()
                .take(3)
                .enumerate()
                .map(|(index, value)| i64::from(value - b'0') * [100, 10, 1][index])
                .sum()
        };
        (hour, minute, second, millis)
    };
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * adjusted_month + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    u64::try_from(days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1000 + millis)
        .ok()
}
