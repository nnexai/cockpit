//! Process-wide pacing. No asynchronous wait holds the state mutex.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cockpit_core::sources::lane::{BackgroundPolicy, RequestLane};
use parking_lot::Mutex;
use reqwest::header::HeaderMap;
use tokio::sync::Notify;
use tokio::time::Instant;
use url::Url;

const HOUR: Duration = Duration::from_secs(3600);
pub(super) const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

pub(super) fn origin(url: &Url) -> Arc<Pacer> {
    static ORIGINS: LazyLock<Mutex<HashMap<String, Arc<Pacer>>>> = LazyLock::new(Mutex::default);
    let key = format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or(0)
    );
    let mut origins = ORIGINS.lock();
    origins
        .entry(key)
        .or_insert_with(|| Arc::new(Pacer::default()))
        .clone()
}

#[derive(Debug, Default)]
pub(super) struct Pacer {
    state: Mutex<State>,
    changed: Notify,
}

#[derive(Debug, Default)]
struct State {
    interactive: u32,
    interactive_waiting: u32,
    background: u32,
    next_background: Option<Instant>,
    background_requests: VecDeque<Instant>,
    cooldown: Option<Instant>,
    cooldown_ms: Option<i64>,
    budget_ms: Option<i64>,
    server_interval: Option<Duration>,
    jitter_sequence: u64,
}

#[derive(Debug)]
pub(super) struct Permit {
    pacer: Arc<Pacer>,
    lane: RequestLane,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.pacer.state.lock();
        match self.lane {
            RequestLane::Interactive => state.interactive -= 1,
            RequestLane::Background => state.background -= 1,
        }
        drop(state);
        self.pacer.changed.notify_waiters();
    }
}

struct WaitingInteractive(Option<Arc<Pacer>>);

impl Drop for WaitingInteractive {
    fn drop(&mut self) {
        if let Some(pacer) = &self.0 {
            pacer.state.lock().interactive_waiting -= 1;
            pacer.changed.notify_waiters();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Grant,
    Wait(Option<Instant>),
    Blocked,
}

impl State {
    fn remaining_requests(&mut self, policy: BackgroundPolicy, now: Instant) -> u32 {
        while self
            .background_requests
            .front()
            .is_some_and(|sent| now.duration_since(*sent) >= HOUR)
        {
            self.background_requests.pop_front();
        }
        let used = u32::try_from(self.background_requests.len()).unwrap_or(u32::MAX);
        policy.hourly_request_cap.max(1).saturating_sub(used)
    }

    fn decide(&mut self, lane: RequestLane, policy: BackgroundPolicy, now: Instant) -> Decision {
        if self.cooldown.is_none() && self.cooldown_ms.is_some_and(|until| until > wall_ms()) {
            // A safe wall deadline may exceed this platform's Instant range.
            return Decision::Blocked;
        }
        if let Some(until) = self.cooldown.filter(|until| *until > now) {
            return if until - now > MAX_RETRY_WAIT {
                Decision::Blocked
            } else {
                Decision::Wait(Some(until))
            };
        }
        match lane {
            RequestLane::Interactive => {
                if self.interactive >= 8 {
                    return Decision::Wait(None);
                }
                self.interactive += 1;
            }
            RequestLane::Background => {
                if self.remaining_requests(policy, now) == 0 {
                    let remaining = HOUR.saturating_sub(
                        now.duration_since(*self.background_requests.front().unwrap()),
                    );
                    self.budget_ms = Some(wall_ms().saturating_add(duration_ms(remaining)));
                    return Decision::Blocked;
                }
                self.budget_ms = None;
                if self.interactive > 0
                    || self.interactive_waiting > 0
                    || self.background >= policy.in_flight.max(1)
                {
                    return Decision::Wait(None);
                }
                if let Some(next) = self.next_background.filter(|next| *next > now) {
                    return Decision::Wait(Some(next));
                }
                let configured =
                    Duration::from_secs_f64(1.0 / f64::from(policy.requests_per_second.max(1)));
                let interval = self
                    .server_interval
                    .map_or(configured, |server| configured.max(server));
                self.next_background = Some(now + interval);
                self.background_requests.push_back(now);
                self.background += 1;
            }
        }
        Decision::Grant
    }
}

impl Pacer {
    pub(super) async fn acquire(
        self: &Arc<Self>,
        lane: RequestLane,
        policy: BackgroundPolicy,
    ) -> Result<Permit, ()> {
        let _waiting = WaitingInteractive(if lane == RequestLane::Interactive {
            self.state.lock().interactive_waiting += 1;
            Some(self.clone())
        } else {
            None
        });
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let decision = self.state.lock().decide(lane, policy, Instant::now());
            match decision {
                Decision::Grant => {
                    return Ok(Permit {
                        pacer: self.clone(),
                        lane,
                    });
                }
                Decision::Blocked => return Err(()),
                Decision::Wait(Some(until)) => {
                    tokio::select! { _ = &mut changed => {}, _ = tokio::time::sleep_until(until) => {} }
                }
                Decision::Wait(None) => changed.await,
            }
        }
    }

    pub(super) fn blocked_until_ms(&self) -> Option<i64> {
        let state = self.state.lock();
        state
            .cooldown_ms
            .into_iter()
            .chain(state.budget_ms)
            .filter(|until| *until > wall_ms())
            .max()
    }

    pub(super) fn remaining_requests(&self, policy: BackgroundPolicy) -> u32 {
        self.state.lock().remaining_requests(policy, Instant::now())
    }

    pub(super) fn observe(&self, headers: &HeaderMap) {
        let number = |name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
        };
        if let (Some(fill), Some(interval)) = (
            number("x-ratelimit-fillrate"),
            number("x-ratelimit-interval-seconds"),
        ) {
            // Retain the conservative learned DC limit across both providers.
            let seconds = (2.0 * interval / fill).clamp(0.001, 3600.0);
            let interval = Duration::from_secs_f64(seconds);
            let mut state = self.state.lock();
            state.server_interval = Some(
                state
                    .server_interval
                    .map_or(interval, |previous| previous.max(interval)),
            );
            if let Some(last) = state.background_requests.back().copied() {
                state.next_background =
                    Some(state.next_background.unwrap_or(last).max(last + interval));
            }
        }
    }

    pub(super) fn throttle(&self, headers: &HeaderMap, retry: u32) -> Duration {
        let floor = headers
            .get("retry-after")
            .and_then(|header| header.to_str().ok())
            .and_then(|value| retry_after(value, SystemTime::now()))
            .unwrap_or_else(|| Duration::from_secs((2u64 << retry.min(5)).min(60)));
        let mut state = self.state.lock();
        // Per-origin bounded jitter, without tokens/addresses in diagnostics.
        state.jitter_sequence = state.jitter_sequence.wrapping_add(1);
        let jitter =
            10 + (wall_ms() as u64 ^ state.jitter_sequence.wrapping_mul(0x9e3779b97f4a7c15)) % 21;
        let wait = floor
            .saturating_add((floor / 100) * jitter as u32)
            .max(Duration::from_millis(100));
        let now = Instant::now();
        let deadline_ms = wall_ms().saturating_add(duration_ms(wait));
        let until = now.checked_add(wait);
        let extends = match until {
            None => true,
            Some(next) => match state.cooldown {
                Some(previous) => next > previous,
                None => state
                    .cooldown_ms
                    .is_none_or(|previous| previous <= wall_ms()),
            },
        };
        if extends {
            state.cooldown = until;
            state.cooldown_ms = Some(
                state
                    .cooldown_ms
                    .map_or(deadline_ms, |previous| previous.max(deadline_ms)),
            );
        }
        let remaining = state
            .cooldown
            .map_or(wait, |until| until.saturating_duration_since(now));
        drop(state);
        self.changed.notify_waiters();
        remaining
    }
}

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn wall_ms() -> i64 {
    duration_ms(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

/// Strict IMF-fixdate (the HTTP sender format) or unsigned delta seconds.
/// Unsafe, malformed and overflowing values are never used as deadlines.
fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse::<u64>().ok().map(Duration::from_secs);
    }
    let mut parts = value.split_ascii_whitespace();
    let weekday = parts.next()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    let month_text = parts.next()?;
    let year = parts.next()?.parse::<i64>().ok()?;
    let time_text = parts.next()?;
    if !matches!(
        weekday,
        "Mon," | "Tue," | "Wed," | "Thu," | "Fri," | "Sat," | "Sun,"
    ) || parts.next()? != "GMT"
        || parts.next().is_some()
    {
        return None;
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|month| *month == month_text)? as u32
        + 1;
    if !(1970..=9999).contains(&year) {
        return None;
    }
    let mut time = time_text.split(':');
    let hour = time.next()?.parse::<u32>().ok()?;
    let minute = time.next()?.parse::<u32>().ok()?;
    let second = time.next()?.parse::<u32>().ok()?;
    if time.next().is_some() {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][month as usize - 1];
    if day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    // Gregorian civil date to days since the Unix epoch.
    let year = year - i64::from(month <= 2);
    let era = year / 400;
    let year_of_era = year - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let days = era * 146097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year
        - 719468;
    let seconds =
        u64::try_from(days * 86400 + i64::from(hour * 3600 + minute * 60 + second)).ok()?;
    let deadline = UNIX_EPOCH.checked_add(Duration::from_secs(seconds))?;
    Some(deadline.duration_since(now).unwrap_or_default())
}

#[cfg(test)]
mod tests;
