use super::*;

#[test]
fn same_origin_registry_ignores_context_and_default_port_but_not_scheme() {
    let jira = origin(&Url::parse("https://SYNC-PACER.example.test/jira").unwrap());
    let wiki = origin(&Url::parse("https://sync-pacer.example.test:443/wiki").unwrap());
    assert!(Arc::ptr_eq(&jira, &wiki));
    assert!(!Arc::ptr_eq(
        &jira,
        &origin(&Url::parse("http://sync-pacer.example.test/wiki").unwrap())
    ));
}

#[test]
fn background_spacing_inflight_priority_and_rolling_budget_are_independent() {
    let now = Instant::now();
    let policy = BackgroundPolicy {
        hourly_request_cap: 3,
        ..Default::default()
    };
    let mut state = State::default();
    assert_eq!(
        state.decide(RequestLane::Background, policy, now),
        Decision::Grant
    );
    assert_eq!(
        state.decide(RequestLane::Background, policy, now),
        Decision::Wait(Some(now + Duration::from_secs(1)))
    );
    assert_eq!(
        state.decide(
            RequestLane::Background,
            policy,
            now + Duration::from_secs(1)
        ),
        Decision::Grant
    );
    assert_eq!(
        state.decide(
            RequestLane::Background,
            policy,
            now + Duration::from_secs(2)
        ),
        Decision::Wait(None)
    );
    state.background -= 1;
    state.interactive_waiting = 1;
    assert_eq!(
        state.decide(
            RequestLane::Background,
            policy,
            now + Duration::from_secs(2)
        ),
        Decision::Wait(None)
    );
    assert_eq!(
        state.decide(
            RequestLane::Interactive,
            policy,
            now + Duration::from_secs(2)
        ),
        Decision::Grant
    );
    state.interactive_waiting = 0;
    assert_eq!(
        state.decide(
            RequestLane::Background,
            policy,
            now + Duration::from_secs(2)
        ),
        Decision::Wait(None)
    );
    state.interactive = 0;
    assert_eq!(
        state.decide(
            RequestLane::Background,
            policy,
            now + Duration::from_secs(2)
        ),
        Decision::Grant
    );
    state.background = 0;
    assert_eq!(
        state.decide(
            RequestLane::Background,
            policy,
            now + Duration::from_secs(3)
        ),
        Decision::Blocked
    );
    assert_eq!(
        state.decide(
            RequestLane::Interactive,
            policy,
            now + Duration::from_secs(3)
        ),
        Decision::Grant
    );
    state.interactive = 0;
    assert_eq!(
        state.decide(RequestLane::Background, policy, now + HOUR),
        Decision::Grant
    );
    assert_eq!(state.background_requests.len(), 3);
}

#[test]
fn cooldown_is_shared_never_shortened_and_preserves_header_floor() {
    let pacer = Pacer::default();
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", "120".parse().unwrap());
    let wait = pacer.throttle(&headers, 0);
    assert!(wait >= Duration::from_secs(132));
    assert!(wait <= Duration::from_secs(156));
    let first = pacer.blocked_until_ms().unwrap();
    headers.insert("retry-after", "1".parse().unwrap());
    pacer.throttle(&headers, 0);
    assert_eq!(pacer.blocked_until_ms(), Some(first));
    let mut state = pacer.state.lock();
    let until = state.cooldown.unwrap();
    for lane in [RequestLane::Interactive, RequestLane::Background] {
        assert_eq!(
            state.decide(
                lane,
                BackgroundPolicy::default(),
                until - Duration::from_secs(61)
            ),
            Decision::Blocked
        );
        assert_eq!(
            state.decide(
                lane,
                BackgroundPolicy::default(),
                until - Duration::from_secs(1)
            ),
            Decision::Wait(Some(until))
        );
    }
    assert_eq!(
        state.decide(RequestLane::Interactive, BackgroundPolicy::default(), until),
        Decision::Grant
    );
}

#[test]
fn retry_after_dates_seconds_and_invalid_values_are_safe() {
    let now = UNIX_EPOCH + Duration::from_secs(1_445_412_470);
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:28:00 GMT", now),
        Some(Duration::from_secs(10))
    );
    assert_eq!(
        retry_after("Wed, 21 Oct 2015 07:27:00 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(retry_after(" 15 ", now), Some(Duration::from_secs(15)));
    assert_eq!(
        retry_after("99999999", now),
        Some(Duration::from_secs(99_999_999))
    );
    for value in [
        "-1",
        "1.5",
        "18446744073709551616",
        "Wed, 31 Feb 2015 07:28:00 GMT",
        "Wed, 21 Oct 2015 25:28:00 GMT",
        "Wed, 21 Oct 2015 07:28:00 UTC",
        "Wed, 21 Oct 2015 07:28:00 GMT private",
    ] {
        assert_eq!(retry_after(value, now), None, "{value}");
    }
}

#[test]
fn advertised_dc_fill_rate_reduces_background_but_not_interactive_rate() {
    let pacer = Pacer::default();
    let mut headers = HeaderMap::new();
    headers.insert("x-ratelimit-fillrate", "30".parse().unwrap());
    headers.insert("x-ratelimit-interval-seconds", "60".parse().unwrap());
    pacer.observe(&headers);
    let now = Instant::now();
    let mut state = pacer.state.lock();
    assert_eq!(
        state.decide(RequestLane::Background, BackgroundPolicy::default(), now),
        Decision::Grant
    );
    assert_eq!(state.next_background, Some(now + Duration::from_secs(4)));
    assert_eq!(
        state.decide(RequestLane::Interactive, BackgroundPolicy::default(), now),
        Decision::Grant
    );
}

#[tokio::test]
async fn interactive_bypasses_background_backlog_and_cancellation_releases_waiters() {
    let pacer = Arc::new(Pacer::default());
    pacer.state.lock().next_background = Some(Instant::now() + HOUR);
    let queued = pacer.clone();
    let background = tokio::spawn(async move {
        queued
            .acquire(RequestLane::Background, BackgroundPolicy::default())
            .await
    });
    tokio::task::yield_now().await;
    assert!(!background.is_finished());
    let manual = pacer
        .acquire(RequestLane::Interactive, BackgroundPolicy::default())
        .await
        .unwrap();
    assert_eq!(pacer.state.lock().interactive, 1);
    drop(manual);
    background.abort();
    assert!(background.await.unwrap_err().is_cancelled());
    assert_eq!(pacer.state.lock().background, 0);

    let mut manual_permits = Vec::new();
    for _ in 0..8 {
        manual_permits.push(
            pacer
                .acquire(RequestLane::Interactive, BackgroundPolicy::default())
                .await
                .unwrap(),
        );
    }
    let queued = pacer.clone();
    let manual = tokio::spawn(async move {
        queued
            .acquire(RequestLane::Interactive, BackgroundPolicy::default())
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while pacer.state.lock().interactive_waiting == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pacer.state.lock().interactive_waiting, 1);
    manual.abort();
    assert!(manual.await.unwrap_err().is_cancelled());
    assert_eq!(pacer.state.lock().interactive_waiting, 0);
    drop(manual_permits);
    assert_eq!(pacer.state.lock().interactive, 0);
}

#[test]
fn unrepresentably_large_retry_after_remains_a_block_instead_of_panicking_or_retrying() {
    let pacer = Pacer::default();
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", u64::MAX.to_string().parse().unwrap());
    assert!(pacer.throttle(&headers, 0) > MAX_RETRY_WAIT);
    assert_eq!(pacer.blocked_until_ms(), Some(i64::MAX));
    let mut state = pacer.state.lock();
    for lane in [RequestLane::Interactive, RequestLane::Background] {
        assert_eq!(
            state.decide(lane, BackgroundPolicy::default(), Instant::now()),
            Decision::Blocked
        );
    }
}

#[test]
fn remaining_budget_reports_consumption_without_charging_and_expires_old_requests() {
    let now = Instant::now();
    let policy = BackgroundPolicy {
        hourly_request_cap: 3,
        ..Default::default()
    };
    let mut state = State::default();
    assert_eq!(state.remaining_requests(policy, now), 3);
    state.decide(RequestLane::Background, policy, now);
    assert_eq!(state.remaining_requests(policy, now), 2);
    assert_eq!(state.remaining_requests(policy, now), 2);
    assert_eq!(
        state.remaining_requests(policy, now + HOUR - Duration::from_nanos(1)),
        2
    );
    assert_eq!(state.remaining_requests(policy, now + HOUR), 3);
    assert_eq!(state.background_requests.len(), 0);
}
