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
        fixture.payload(omp_payload(NOW));
        fixture.script("");
        fixture
    }
    fn payload(&self, payload: Value) {
        fs::write(
            self.root.join("omp.json"),
            serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();
    }
    fn script(&self, behavior: &str) {
        let content = format!(
            "#!/bin/sh\nprintf x >> '{}'\n{behavior}\ncat '{}'\n",
            self.root.join("omp.calls").display(),
            self.root.join("omp.json").display()
        );
        let path = self.root.join("omp");
        fs::write(&path, content).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn service(&self, now: u64) -> Arc<QuotaService> {
        let service = Arc::new(QuotaService::new(
            QuotaConfiguration {
                omp_executable: self.root.join("omp"),
            },
            &self.root,
        ));
        service.now.store(now, Ordering::Relaxed);
        service
    }
    fn count(&self) -> usize {
        fs::read(self.root.join("omp.calls"))
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
        {"provider":"openai-codex","fetchedAt":now,"metadata":{"email":"private-marker@example.test"},"limits":[
            {"id":"private-marker","scope":{"accountId":"private-marker","tier":"personal-secret"},"window":{"id":"private-marker","durationMs":18000000},"amount":{"unit":"percent","usedFraction":0.25}},
            {"window":{"id":"weekly"},"amount":{"unit":"percent","usedFraction":0.4}}
        ]},
        {"provider":"anthropic","fetchedAt":now,"limits":[{"scope":{"tier":"opus"},"window":{"id":"7d"},"amount":{"unit":"percent","used":90,"limit":100}}]},
        {"provider":"github-copilot","fetchedAt":now-1000,"accountId":"private-marker","limits":[
            {"id":"copilot:premium","label":"Premium Requests","scope":{"tier":"business","accountId":"private-marker"},"window":{"id":"monthly"},"amount":{"unit":"requests","used":4,"limit":8000,"remaining":7996,"usedFraction":0.0005,"remainingFraction":0.9995}}
        ]}
    ],"accountsWithoutUsage":[],"disabledCredentials":[{"token":"private-marker"}]})
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
async fn source_projection_is_anonymous_and_preserves_real_windows_and_business_counters() {
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
    let codex = &value.providers[0].accounts[0].limits;
    assert_eq!(codex[0].id, "codex:5h:0");
    assert_eq!(codex[0].window.as_deref(), Some("5h"));
    assert_eq!(codex[0].used_fraction, Some(0.25));
    assert_eq!(codex[0].tier, None);
    assert_eq!(codex[1].window.as_deref(), Some("weekly"));
    assert_eq!(codex[1].used_fraction, Some(0.4));
    let claude = &value.providers[1].accounts[0].limits[0];
    assert_eq!(claude.tier.as_deref(), Some("opus"));
    assert_eq!(claude.used_fraction, Some(0.9));
    let copilot = &value.providers[2].accounts[0].limits[0];
    assert_eq!(
        (copilot.used, copilot.remaining, copilot.limit),
        (Some(4.0), Some(7996.0), Some(8000.0))
    );
    assert_eq!(copilot.unit, QuotaUnit::Credits);
    assert_eq!(copilot.window.as_deref(), Some("monthly"));
    assert_eq!(copilot.tier, None);
    assert_eq!(copilot.used_fraction, Some(0.0005));
    assert_eq!(value.providers[2].fetched_at_ms, Some(NOW - 1000));
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
async fn shared_schedule_and_failure_backoff_keep_all_last_good_visibly_stale() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    let initial = settled(&service).await;
    for now in [NOW + 60_000, NOW + CADENCE - 1] {
        service.now.store(now, Ordering::Relaxed);
        settled(&service).await;
        assert_eq!(fixture.count(), 1);
    }
    fixture.script("printf 'private-marker failure' >&2; exit 1");
    for (offset, count) in [
        (CADENCE, 2),
        (2 * CADENCE, 3),
        (3 * CADENCE, 3),
        (4 * CADENCE, 4),
    ] {
        service.now.store(NOW + offset, Ordering::Relaxed);
        let value = settled(&service).await;
        assert_eq!(fixture.count(), count);
        for (provider, previous) in value.providers.iter().zip(&initial.providers) {
            assert_eq!(provider.accounts, previous.accounts);
            assert_eq!(provider.fetched_at_ms, previous.fetched_at_ms);
            assert_eq!(provider.error, Some(QuotaErrorCode::Failed));
            assert!(provider.stale);
        }
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
    fixture.script("sleep 0.1");
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
    assert_eq!(fixture.count(), 1);
    assert_eq!(va2.providers, vb2.providers);
    assert!(!va.collecting && !vb.collecting);
    assert!(
        va2.providers
            .iter()
            .all(|provider| provider.state == QuotaProviderState::Available)
    );
}

#[tokio::test]
async fn hostile_or_malformed_cache_has_no_authority() {
    let fixture = Fixture::new();
    settled(&fixture.service(NOW)).await;
    let mut stored: Value = serde_json::from_slice(&fs::read(fixture.cache()).unwrap()).unwrap();
    stored["omp"]["providers"][0]["accounts"][0]["limits"][0]["id"] = json!("private-marker");
    fs::write(fixture.cache(), serde_json::to_vec(&stored).unwrap()).unwrap();
    let value = settled(&fixture.service(NOW)).await;
    assert_eq!(value.providers[0].accounts[0].limits[0].id, "codex:5h:0");
    assert_eq!(fixture.count(), 2);
    fs::write(fixture.cache(), "{broken").unwrap();
    assert_eq!(
        settled(&fixture.service(NOW)).await.providers[0].state,
        QuotaProviderState::Available
    );
    assert_eq!(fixture.count(), 3);
}

#[tokio::test]
async fn obsolete_split_source_cache_cannot_supply_copilot_values_or_a_lease() {
    let fixture = Fixture::new();
    settled(&fixture.service(NOW)).await;
    let mut stored: Value = serde_json::from_slice(&fs::read(fixture.cache()).unwrap()).unwrap();
    stored["schema"] = json!(1);
    let copilot = stored["omp"]["providers"]
        .as_array_mut()
        .unwrap()
        .pop()
        .unwrap();
    stored["gh"] = json!({"attempted_at_ms":NOW,"next_attempt_at_ms":NOW+CADENCE,"failures":0,"providers":[copilot]});
    stored["gh"]["providers"][0]["accounts"][0]["limits"][0]["used"] = json!(7000);
    fs::write(fixture.cache(), serde_json::to_vec(&stored).unwrap()).unwrap();
    let value = settled(&fixture.service(NOW)).await;
    assert_eq!(value.providers[2].accounts[0].limits[0].used, Some(4.0));
    assert_eq!(fixture.count(), 2);
}

#[tokio::test]
async fn symlinked_root_lock_or_snapshot_never_runs_source() {
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
        assert_eq!(fixture.count(), 0);
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
    for (provider, previous) in value.providers.iter().zip(&initial.providers) {
        assert_eq!(provider.accounts, previous.accounts);
        assert!(provider.error == Some(QuotaErrorCode::CacheUnavailable) && provider.stale);
    }
    service.now.store(NOW + CADENCE + 60_000, Ordering::Relaxed);
    settled(&service).await;
    assert_eq!(fixture.count(), 1);
}

#[test]
fn copilot_report_freshness_has_inclusive_bounds_and_rejects_expired_balances() {
    for fetched_at in [NOW + 60_000, NOW - RETAIN] {
        let mut payload = omp_payload(NOW);
        payload["reports"][2]["fetchedAt"] = json!(fetched_at);
        let providers = parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        assert_eq!(providers[2].state, QuotaProviderState::Available);
        assert_eq!(providers[2].error, None);
        assert_eq!(providers[2].accounts[0].fetched_at_ms, fetched_at);
        assert_eq!(providers[2].accounts[0].limits[0].remaining, Some(7996.0));
    }
    let mut payload = omp_payload(NOW);
    payload["reports"][2]["fetchedAt"] = json!(NOW + 60_001);
    assert_eq!(
        parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).err(),
        Some(QuotaErrorCode::Malformed)
    );
    payload["reports"][2]["fetchedAt"] = json!(NOW - RETAIN - 1);
    let providers = parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
    assert_eq!(providers[2].state, QuotaProviderState::Unavailable);
    assert_eq!(providers[2].error, Some(QuotaErrorCode::UsageUnavailable));
    assert!(providers[2].accounts.is_empty());
}

#[test]
fn copilot_source_units_do_not_scale_counters_or_invent_unknown_entitlement() {
    for unit in ["requests", "credits"] {
        let mut payload = omp_payload(NOW);
        let amount = &mut payload["reports"][2]["limits"][0]["amount"];
        *amount = json!({"unit":unit,"used":4,"limit":8000,"remaining":7996});
        let providers = parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        let limit = &providers[2].accounts[0].limits[0];
        assert_eq!(limit.unit, QuotaUnit::Credits);
        assert_eq!(
            (limit.used, limit.limit, limit.remaining),
            (Some(4.0), Some(8000.0), Some(7996.0))
        );
        assert_eq!(limit.used_fraction, Some(0.0005));
    }
    for (used, total, fraction) in [
        (0.0, 8000.0, Some(0.0)),
        (8000.0, 8000.0, Some(1.0)),
        (0.0, 0.0, None),
    ] {
        let mut payload = omp_payload(NOW);
        payload["reports"][2]["limits"][0]["amount"] =
            json!({"unit":"requests","used":used,"limit":total});
        let providers = parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        let limit = &providers[2].accounts[0].limits[0];
        assert_eq!(limit.used_fraction, fraction);
        assert_eq!(limit.used, Some(used));
        assert_eq!(limit.limit, Some(total));
    }
}

#[test]
fn malformed_copilot_counters_are_rejected_instead_of_persisted() {
    for field in ["used", "limit", "remaining"] {
        let mut payload = omp_payload(NOW);
        payload["reports"][2]["limits"][0]["amount"][field] = json!(-1);
        assert_eq!(
            parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).err(),
            Some(QuotaErrorCode::Malformed)
        );
    }
}

#[test]
fn copilot_unlimited_is_distinct_from_unknown_or_zero() {
    for (notes, used, expected_unlimited) in [
        (json!(["Unlimited"]), None, true),
        (json!([]), None, false),
        (json!(["unlimited"]), None, false),
        (json!(["Unlimited"]), Some(0.0), false),
    ] {
        let mut payload = omp_payload(NOW);
        let input = &mut payload["reports"][2]["limits"][0];
        input["notes"] = notes;
        input["amount"] = json!({"unit":"requests","used":used});
        let providers = parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
        let limit = &providers[2].accounts[0].limits[0];
        assert_eq!(limit.unlimited, expected_unlimited);
        assert_eq!(limit.used_fraction, None);
        assert_eq!(limit.used, used);
        assert_eq!(limit.limit, None);
        assert_eq!(limit.remaining, None);
    }
}

#[tokio::test]
async fn unrelated_or_unsupported_copilot_counters_remove_previous_balance() {
    for (id, unit) in [
        ("copilot:chat", "requests"),
        ("copilot:completions", "requests"),
        ("copilot:model:private-marker", "requests"),
        ("copilot:premium", "tokens"),
        ("copilot:premium", "percent"),
    ] {
        let fixture = Fixture::new();
        let service = fixture.service(NOW);
        assert_eq!(
            settled(&service).await.providers[2].state,
            QuotaProviderState::Available
        );
        let mut payload = omp_payload(NOW + CADENCE);
        payload["reports"][2]["limits"][0]["id"] = json!(id);
        payload["reports"][2]["limits"][0]["amount"]["unit"] = json!(unit);
        fixture.payload(payload);
        service.now.store(NOW + CADENCE, Ordering::Relaxed);
        let value = settled(&service).await;
        assert_eq!(value.providers[2].state, QuotaProviderState::Unsupported);
        assert_eq!(value.providers[2].error, Some(QuotaErrorCode::Unsupported));
        assert!(value.providers[2].accounts.is_empty());
        assert_eq!(value.providers[2].fetched_at_ms, None);
        assert_eq!(value.providers[0].state, QuotaProviderState::Available);
        assert_eq!(value.providers[1].state, QuotaProviderState::Available);
    }
}

#[test]
fn absent_copilot_report_is_not_signed_in_instead_of_an_unknown_balance() {
    let mut payload = omp_payload(NOW);
    payload["reports"].as_array_mut().unwrap().pop();
    let providers = parse::omp(&serde_json::to_vec(&payload).unwrap(), NOW).unwrap();
    assert_eq!(providers[2].state, QuotaProviderState::NotSignedIn);
    assert_eq!(providers[2].error, Some(QuotaErrorCode::NotSignedIn));
    assert!(providers[2].accounts.is_empty());
}

#[tokio::test]
async fn missing_source_and_failed_command_are_sanitized_for_all_providers() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.join("omp")).unwrap();
    let value = settled(&fixture.service(NOW)).await;
    assert!(
        value
            .providers
            .iter()
            .all(|provider| provider.error == Some(QuotaErrorCode::SourceMissing))
    );
    fixture.script("printf 'private-marker' >&2; exit 1");
    let value = settled(&fixture.service(NOW + CADENCE)).await;
    assert!(
        value
            .providers
            .iter()
            .all(|provider| provider.error == Some(QuotaErrorCode::Failed))
    );
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
    payload["reports"].as_array_mut().unwrap().extend([
        json!({"provider":"anthropic","fetchedAt":NOW-1000,"limits":[
            {"scope":{"tier":"private-marker"},"window":{"id":"7d"},"amount":{"unit":"percent"}},
            {"scope":{"modelId":"claude-sonnet-4"},"window":{"id":"5h"},"amount":{"unit":"percent","remainingFraction":0.2}}
        ]}),
        json!({"provider":"github-copilot","accountId":"private-marker","fetchedAt":NOW-2000,"limits":[
            {"id":"copilot:premium","window":{"id":"monthly"},"amount":{"unit":"requests","used":2,"limit":4000,"remaining":3998}}
        ]})
    ]);
    fixture.payload(payload);
    let value = settled(&fixture.service(NOW)).await;
    let claude = &value.providers[1];
    assert_eq!(claude.fetched_at_ms, Some(NOW - 1000));
    assert_eq!(claude.accounts[1].limits[0].used_fraction, None);
    assert_eq!(claude.accounts[1].limits[0].tier, None);
    assert_eq!(claude.accounts[1].limits[0].level, QuotaLevel::Unknown);
    assert_eq!(claude.accounts[1].limits[1].used_fraction, Some(0.8));
    assert_eq!(claude.accounts[1].limits[1].tier.as_deref(), Some("sonnet"));
    assert_eq!(claude.accounts[0].limits[0].tier.as_deref(), Some("opus"));
    let copilot = &value.providers[2];
    assert_eq!(copilot.fetched_at_ms, Some(NOW - 2000));
    assert_eq!(copilot.accounts.len(), 2);
    assert_eq!(copilot.accounts[1].limits[0].used, Some(2.0));
    assert_eq!(copilot.accounts[1].limits[0].used_fraction, Some(0.0005));
    assert!(
        !serde_json::to_string(&value)
            .unwrap()
            .contains("private-marker")
    );
}

#[tokio::test]
async fn unavailable_provider_retains_previous_report_time_without_appearing_fresh() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    let initial = settled(&service).await;
    let mut payload = omp_payload(NOW + CADENCE);
    payload["reports"].as_array_mut().unwrap().pop();
    payload["accountsWithoutUsage"] =
        json!([{"provider":"github-copilot","accountId":"private-marker"}]);
    fixture.payload(payload);
    service.now.store(NOW + CADENCE, Ordering::Relaxed);
    let value = settled(&service).await;
    assert_eq!(value.providers[2].accounts, initial.providers[2].accounts);
    assert_eq!(value.providers[2].fetched_at_ms, Some(NOW - 1000));
    assert_eq!(
        value.providers[2].error,
        Some(QuotaErrorCode::UsageUnavailable)
    );
    assert!(value.providers[2].stale);
    assert!(!value.providers[0].stale);
}

#[tokio::test]
async fn expired_last_good_is_removed_without_erasing_latest_failure() {
    let fixture = Fixture::new();
    let service = fixture.service(NOW);
    settled(&service).await;
    fixture.script("exit 1");
    service.now.store(NOW + RETAIN + 1, Ordering::Relaxed);
    let value = settled(&service).await;
    assert!(value.providers.iter().all(|provider| provider.state
        == QuotaProviderState::Unavailable
        && provider.error == Some(QuotaErrorCode::Failed)
        && provider.accounts.is_empty()
        && provider.fetched_at_ms.is_none()
        && !provider.stale));
}
