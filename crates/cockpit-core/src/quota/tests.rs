#![cfg(unix)]
use super::*;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::Ordering,
};

const NOW: u64 = 1_790_856_600_000;
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-quota-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let fixture = Self { root };
        fixture.payload("omp", omp_payload(NOW));
        fixture.payload("gh", credits());
        fixture.script("omp", "");
        fixture.script("gh", "");
        fixture
    }
    fn payload(&self, source: &str, payload: Value) {
        fs::write(
            self.root.join(format!("{source}.json")),
            serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();
    }
    fn script(&self, source: &str, behavior: &str) {
        let content = format!(
            "#!/bin/sh\nprintf x >> '{}'
{behavior}
cat '{}'
",
            self.root.join(format!("{source}.calls")).display(),
            self.root.join(format!("{source}.json")).display()
        );
        let path = self.root.join(source);
        fs::write(&path, content).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn service(&self, now: u64) -> Arc<QuotaService> {
        let service = Arc::new(QuotaService::new(
            QuotaConfiguration {
                omp_executable: self.root.join("omp"),
                gh_executable: self.root.join("gh"),
            },
            &self.root,
        ));
        service.now.store(now, Ordering::Relaxed);
        service
    }
    fn count(&self, source: &str) -> usize {
        fs::read(self.root.join(format!("{source}.calls")))
            .unwrap_or_default()
            .len()
    }
    fn cache(&self) -> PathBuf {
        self.root.join("quota/v1/snapshot.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn omp_payload(now: u64) -> Value {
    json!({"reports":[
        {"provider":"openai-codex","fetchedAt":now,"metadata":{"email":"private-marker@example.test"},"limits":[{"id":"private-marker","scope":{"accountId":"private-marker","tier":"personal-secret"},"window":{"id":"private-marker","durationMs":18000000},"amount":{"unit":"percent","usedFraction":0.25}}]},
        {"provider":"anthropic","fetchedAt":now,"limits":[{"scope":{"tier":"opus"},"window":{"id":"7d"},"amount":{"unit":"percent","used":90,"limit":100}}]},
        {"provider":"github-copilot","fetchedAt":now,"limits":[{"amount":{"unit":"requests","used":99,"limit":100}}]}
    ],"accountsWithoutUsage":[],"disabledCredentials":[{"token":"private-marker"}]})
}
fn credits() -> Value {
    json!({"token_based_billing":true,"quota_reset_date_utc":"2026-11-01","login":"private-marker","quota_snapshots":{"premium_interactions":{"token_based_billing":true,"credits_used":250,"remaining":1250,"entitlement":1500,"unlimited":false,"timestamp_utc":"2026-10-01T05:29:00Z"}}})
}
async fn settled(service: &Arc<QuotaService>) -> QuotaStatusResponse {
    for _ in 0..300 {
        let value = service.status().await;
        if !value.collecting {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("quota collection did not settle");
}

#[tokio::test]
async fn source_projection_is_anonymous_and_preserves_real_windows() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    let first = service.status().await;
    assert!(first.collecting);
    assert!(
        first
            .providers
            .iter()
            .all(|value| value.state == QuotaProviderState::Pending)
    );
    let value = settled(&service).await;
    let codex = &value.providers[0].accounts[0].limits[0];
    assert_eq!(codex.id, "codex:5h:0");
    assert_eq!(codex.window.as_deref(), Some("5h"));
    assert_eq!(codex.used_fraction, Some(0.25));
    assert_eq!(codex.tier, None);
    let claude = &value.providers[1].accounts[0].limits[0];
    assert_eq!(claude.tier.as_deref(), Some("opus"));
    assert_eq!(claude.used_fraction, Some(0.9));
    let copilot = &value.providers[2].accounts[0].limits[0];
    assert_eq!(
        (copilot.used, copilot.remaining, copilot.limit),
        (Some(250.0), Some(1250.0), Some(1500.0))
    );
    assert_eq!(value.providers[2].fetched_at_ms, Some(1_790_832_540_000));
    assert!(
        !serde_json::to_string(&value)
            .unwrap()
            .contains("private-marker")
    );
    assert!(
        !fs::read_to_string(fixture.cache())
            .unwrap()
            .contains("private-marker")
    );
    assert_eq!(
        fs::metadata(fixture.cache()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn both_credit_mode_flags_and_all_numeric_fields_are_required() {
    for mutation in 0..8 {
        let fixture = Fixture::new();
        let mut payload = credits();
        match mutation {
            0 => payload["token_based_billing"] = json!(false),
            1 => {
                payload["quota_snapshots"]["premium_interactions"]["token_based_billing"] =
                    json!(false)
            }
            2 => {
                payload["quota_snapshots"]["premium_interactions"]
                    .as_object_mut()
                    .unwrap()
                    .remove("credits_used");
            }
            3 => payload["quota_snapshots"]["premium_interactions"]["remaining"] = json!(-1),
            4 => payload["quota_snapshots"]["premium_interactions"]["entitlement"] = Value::Null,
            5 => payload["quota_snapshots"]["premium_interactions"]["credits_used"] = json!("250"),
            6 => payload["token_based_billing"] = json!("true"),
            _ => payload["quota_snapshots"]["premium_interactions"]["unlimited"] = Value::Null,
        }
        fixture.payload("gh", payload);
        let value = settled(&fixture.service(NOW)).await;
        assert_eq!(value.providers[2].state, QuotaProviderState::Unsupported);
        assert!(value.providers[2].accounts.is_empty());
    }
}

#[tokio::test]
async fn unsupported_billing_cutover_removes_previous_balance() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    assert_eq!(
        settled(&service).await.providers[2].state,
        QuotaProviderState::Available
    );
    fixture.payload("gh", json!({"token_based_billing":false,"quota_snapshots":{"premium_interactions":{"remaining":99,"entitlement":100,"unlimited":false}}}));
    service.now.store(NOW + CADENCE, Ordering::Relaxed);
    let value = settled(&service).await;
    assert_eq!(value.providers[2].state, QuotaProviderState::Unsupported);
    assert!(value.providers[2].accounts.is_empty());
    assert_eq!(value.providers[2].fetched_at_ms, None);
}

#[tokio::test]
async fn independent_schedules_and_failure_backoff_keep_last_good_visibly_stale() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    let initial = settled(&service).await;
    for now in [NOW + 60_000, NOW + CADENCE - 1] {
        service.now.store(now, Ordering::Relaxed);
        settled(&service).await;
        assert_eq!((fixture.count("omp"), fixture.count("gh")), (1, 1));
    }
    fixture.script("gh", "printf 'private-marker failure' >&2; exit 1");
    for (offset, omp_count, gh_count) in [
        (CADENCE, 2, 2),
        (2 * CADENCE, 3, 3),
        (3 * CADENCE, 4, 3),
        (4 * CADENCE, 5, 4),
    ] {
        service.now.store(NOW + offset, Ordering::Relaxed);
        let value = settled(&service).await;
        assert_eq!(
            (fixture.count("omp"), fixture.count("gh")),
            (omp_count, gh_count)
        );
        assert_eq!(value.providers[2].accounts, initial.providers[2].accounts);
        assert_eq!(value.providers[2].error, Some(QuotaErrorCode::Failed));
        assert!(value.providers[2].stale);
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("private-marker")
        );
    }
}

#[tokio::test]
async fn concurrent_services_share_collection_lease_and_cached_values() {
    let fixture = Fixture::new();
    fixture.script("omp", "sleep 0.1");
    fixture.script("gh", "sleep 0.1");
    let a = fixture.service(NOW);
    let b = fixture.service(NOW);
    tokio::join!(a.status(), b.status());
    let va = settled(&a).await;
    let vb = settled(&b).await;
    // Either host may own the lock. Advance past contention recheck, not source TTL.
    a.now.store(NOW + 3_001, Ordering::Relaxed);
    b.now.store(NOW + 3_001, Ordering::Relaxed);
    let va2 = settled(&a).await;
    let vb2 = settled(&b).await;
    assert_eq!((fixture.count("omp"), fixture.count("gh")), (1, 1));
    assert_eq!(va2.providers, vb2.providers);
    assert!(va.collecting == false && vb.collecting == false);
    assert_eq!(va2.providers[0].state, QuotaProviderState::Available);
}

#[tokio::test]
async fn hostile_or_malformed_cache_has_no_authority() {
    let fixture = Fixture::new();
    let a = fixture.service(NOW);
    settled(&a).await;
    let mut stored: Value = serde_json::from_slice(&fs::read(fixture.cache()).unwrap()).unwrap();
    stored["omp"]["providers"][0]["accounts"][0]["limits"][0]["id"] = json!("private-marker");
    fs::write(fixture.cache(), serde_json::to_vec(&stored).unwrap()).unwrap();
    let value = settled(&fixture.service(NOW)).await;
    assert_eq!(value.providers[0].accounts[0].limits[0].id, "codex:5h:0");
    assert_eq!((fixture.count("omp"), fixture.count("gh")), (2, 2));
    fs::write(fixture.cache(), "{broken").unwrap();
    assert_eq!(
        settled(&fixture.service(NOW)).await.providers[0].state,
        QuotaProviderState::Available
    );
    assert_eq!(fixture.count("omp"), 3);
}

#[tokio::test]
async fn symlinked_root_lock_or_snapshot_never_runs_sources() {
    for target in ["root", "collect.lock", "snapshot.json"] {
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside");
        fs::create_dir(&outside).unwrap();
        if target == "root" {
            fs::create_dir(fixture.root.join("quota")).unwrap();
            symlink(&outside, fixture.root.join("quota/v1")).unwrap();
        } else {
            fs::create_dir_all(fixture.root.join("quota/v1")).unwrap();
            fs::write(outside.join("target"), "private-marker").unwrap();
            symlink(
                outside.join("target"),
                fixture.root.join("quota/v1").join(target),
            )
            .unwrap();
        }
        let value = settled(&fixture.service(NOW)).await;
        assert!(
            value
                .providers
                .iter()
                .all(|value| value.error == Some(QuotaErrorCode::CacheUnavailable))
        );
        assert_eq!((fixture.count("omp"), fixture.count("gh")), (0, 0));
        if target != "root" {
            assert_eq!(
                fs::read_to_string(outside.join("target")).unwrap(),
                "private-marker"
            );
        }
    }
}

#[tokio::test]
async fn unavailable_cache_preserves_previous_values_and_backs_off() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    let initial = settled(&service).await;
    fs::remove_file(fixture.root.join("quota/v1/collect.lock")).unwrap();
    fs::create_dir(fixture.root.join("quota/v1/collect.lock")).unwrap();
    service.now.store(NOW + CADENCE, Ordering::Relaxed);
    let value = settled(&service).await;
    assert_eq!(value.providers[0].accounts, initial.providers[0].accounts);
    assert!(
        value
            .providers
            .iter()
            .all(|value| value.error == Some(QuotaErrorCode::CacheUnavailable) && value.stale)
    );
    service.now.store(NOW + CADENCE + 60_000, Ordering::Relaxed);
    settled(&service).await;
    assert_eq!((fixture.count("omp"), fixture.count("gh")), (1, 1));
}

#[tokio::test]
async fn stale_source_timestamps_are_not_replaced_by_completion_time() {
    let fixture = Fixture::new();
    let mut payload = credits();
    payload["quota_snapshots"]["premium_interactions"]["timestamp_utc"] =
        json!("2026-09-29T00:00:00Z");
    fixture.payload("gh", payload);
    let value = settled(&fixture.service(NOW)).await;
    assert_eq!(value.providers[2].state, QuotaProviderState::Unavailable);
    assert!(value.providers[2].accounts.is_empty());
}

#[tokio::test]
async fn future_source_timestamp_is_malformed_and_unlimited_is_not_zero() {
    let fixture = Fixture::new();
    let mut payload = credits();
    payload["quota_snapshots"]["premium_interactions"]["timestamp_utc"] =
        json!("2099-01-01T00:00:00Z");
    fixture.payload("gh", payload);
    assert_eq!(
        settled(&fixture.service(NOW)).await.providers[2].error,
        Some(QuotaErrorCode::Malformed)
    );
    let fixture = Fixture::new();
    let mut payload = credits();
    payload["quota_snapshots"]["premium_interactions"]["unlimited"] = json!(true);
    fixture.payload("gh", payload);
    let value = settled(&fixture.service(NOW)).await;
    let limit = &value.providers[2].accounts[0].limits[0];
    assert!(limit.unlimited);
    assert_eq!(
        (
            limit.used_fraction,
            limit.used,
            limit.remaining,
            limit.limit
        ),
        (None, None, None, None)
    );
}

#[test]
fn copilot_credit_fraction_distinguishes_zero_usage_from_unknown_entitlement() {
    for (used, remaining, total, fraction, level) in [
        (250.0, 1250.0, 1500.0, Some(250.0 / 1500.0), QuotaLevel::Ok),
        (0.0, 1500.0, 1500.0, Some(0.0), QuotaLevel::Ok),
        (1500.0, 0.0, 1500.0, Some(1.0), QuotaLevel::Exhausted),
        (0.0, 0.0, 0.0, None, QuotaLevel::Unknown),
    ] {
        let mut payload = credits();
        let snapshot = &mut payload["quota_snapshots"]["premium_interactions"];
        snapshot["credits_used"] = json!(used);
        snapshot["remaining"] = json!(remaining);
        snapshot["entitlement"] = json!(total);
        let providers = parse::copilot(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        assert_eq!(providers[0].state, QuotaProviderState::Available);
        let limit = &providers[0].accounts[0].limits[0];
        assert_eq!(
            (limit.used, limit.remaining, limit.limit),
            (Some(used), Some(remaining), Some(total))
        );
        assert_eq!(limit.used_fraction, fraction);
        assert_eq!(limit.level, level);
        assert!(!limit.unlimited);
    }
}

#[test]
fn copilot_source_freshness_has_inclusive_bounds_and_rejects_expired_balances() {
    for (timestamp, fetched_at) in [
        ("2026-10-01T12:11:00.000Z", NOW + 60_000),
        ("2026-09-30T12:10:00.000Z", NOW - RETAIN),
    ] {
        let mut payload = credits();
        payload["quota_snapshots"]["premium_interactions"]["timestamp_utc"] = json!(timestamp);
        let providers = parse::copilot(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        assert_eq!(providers[0].state, QuotaProviderState::Available);
        assert_eq!(providers[0].error, None);
        assert_eq!(providers[0].accounts[0].fetched_at_ms, fetched_at);
        assert_eq!(providers[0].accounts[0].limits[0].remaining, Some(1250.0));
    }
    let mut payload = credits();
    payload["quota_snapshots"]["premium_interactions"]["timestamp_utc"] =
        json!("2026-10-01T12:11:00.001Z");
    assert_eq!(
        parse::copilot(&serde_json::to_vec(&payload).unwrap(), NOW).err(),
        Some(QuotaErrorCode::Malformed)
    );
    for timestamp in ["2026-09-30T12:09:59.999Z", "2026-09-29T00:00:00Z"] {
        payload["quota_snapshots"]["premium_interactions"]["timestamp_utc"] = json!(timestamp);
        let providers = parse::copilot(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        assert_eq!(providers[0].state, QuotaProviderState::Unavailable);
        assert_eq!(providers[0].error, Some(QuotaErrorCode::UsageUnavailable));
        assert!(providers[0].accounts.is_empty());
    }
}

#[tokio::test]
async fn missing_source_and_auth_failure_are_sanitized_and_provider_independent() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.join("omp")).unwrap();
    fixture.script("gh", "printf 'gh auth login private-marker' >&2; exit 4");
    let value = settled(&fixture.service(NOW)).await;
    assert_eq!(
        value.providers[0].error,
        Some(QuotaErrorCode::SourceMissing)
    );
    assert_eq!(
        value.providers[1].error,
        Some(QuotaErrorCode::SourceMissing)
    );
    assert_eq!(value.providers[2].state, QuotaProviderState::NotSignedIn);
    assert_eq!(value.providers[2].error, Some(QuotaErrorCode::NotSignedIn));
    assert!(
        !serde_json::to_string(&value)
            .unwrap()
            .contains("private-marker")
    );
}

#[tokio::test]
async fn multiple_accounts_unknown_amounts_and_unknown_tiers_are_not_lost() {
    let fixture = Fixture::new();
    let mut payload = omp_payload(NOW);
    let second = json!({"provider":"anthropic","fetchedAt":NOW-1000,"limits":[
        {"scope":{"tier":"private-marker"},"window":{"id":"7d"},"amount":{"unit":"percent"}},
        {"scope":{"modelId":"claude-sonnet-4"},"window":{"id":"5h"},"amount":{"unit":"percent","remainingFraction":0.2}}
    ]});
    payload["reports"].as_array_mut().unwrap().push(second);
    fixture.payload("omp", payload);
    let value = settled(&fixture.service(NOW)).await;
    let claude = &value.providers[1];
    assert_eq!(claude.fetched_at_ms, Some(NOW - 1000));
    assert_eq!(claude.accounts[1].limits[0].used_fraction, None);
    assert_eq!(claude.accounts[1].limits[0].tier, None);
    assert_eq!(claude.accounts[1].limits[0].level, QuotaLevel::Unknown);
    assert_eq!(claude.accounts[1].limits[1].used_fraction, Some(0.8));
    assert_eq!(claude.accounts[1].limits[1].tier.as_deref(), Some("sonnet"));
    assert_eq!(claude.accounts[0].limits[0].tier.as_deref(), Some("opus"));
}

#[tokio::test]
async fn expired_last_good_is_removed_without_erasing_latest_failure() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    settled(&service).await;
    fixture.script("omp", "exit 1");
    fixture.script("gh", "exit 1");
    service.now.store(NOW + RETAIN + 1, Ordering::Relaxed);
    let value = settled(&service).await;
    assert!(value.providers.iter().all(|provider| provider.state
        == QuotaProviderState::Unavailable
        && provider.error == Some(QuotaErrorCode::Failed)
        && provider.accounts.is_empty()
        && provider.fetched_at_ms.is_none()
        && !provider.stale));
}
