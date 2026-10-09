//! Task-local source request priority and background server-load policy.
//!
//! Spawned tasks do not inherit Tokio task locals automatically: wrap provider
//! work with `inherit` before spawning it. A missing scope is interactive.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestLane {
    Interactive,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundPolicy {
    pub min_interval_seconds: u32,
    pub in_flight: u32,
    pub hourly_request_cap: u32,
}

impl Default for BackgroundPolicy {
    fn default() -> Self {
        let defaults = crate::config::LibrarySyncConfiguration::default();
        Self {
            min_interval_seconds: defaults.background_min_interval_seconds,
            in_flight: defaults.background_in_flight,
            hourly_request_cap: defaults.hourly_request_cap,
        }
    }
}

#[derive(Clone, Copy)]
struct Context {
    lane: RequestLane,
    policy: BackgroundPolicy,
}

tokio::task_local! {
    static CONTEXT: Context;
}

pub fn current() -> RequestLane {
    CONTEXT
        .try_with(|context| context.lane)
        .unwrap_or(RequestLane::Interactive)
}

pub fn background_policy() -> BackgroundPolicy {
    CONTEXT
        .try_with(|context| context.policy)
        .unwrap_or_default()
}

pub async fn scope<F: Future>(lane: RequestLane, future: F) -> F::Output {
    let context = Context {
        lane,
        policy: background_policy(),
    };
    CONTEXT.scope(context, future).await
}

/// Scope a scheduler turn, retaining its policy through `inherit` spawn wrappers.
pub async fn scope_background<F: Future>(policy: BackgroundPolicy, future: F) -> F::Output {
    CONTEXT
        .scope(
            Context {
                lane: RequestLane::Background,
                policy,
            },
            future,
        )
        .await
}

/// Capture now, not when the returned future is first polled in another task.
pub fn inherit<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let context = Context {
        lane: current(),
        policy: background_policy(),
    };
    async move { CONTEXT.scope(context, future).await }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawned_work_inherits_priority_and_policy_without_leaking_to_caller() {
        let policy = BackgroundPolicy {
            min_interval_seconds: 5,
            in_flight: 4,
            hourly_request_cap: 100,
        };
        let captured = scope_background(policy, async {
            tokio::spawn(inherit(async { (current(), background_policy()) }))
                .await
                .unwrap()
        })
        .await;
        assert_eq!(captured, (RequestLane::Background, policy));
        assert_eq!(current(), RequestLane::Interactive);
        scope_background(policy, async {
            scope(RequestLane::Interactive, async {
                assert_eq!(current(), RequestLane::Interactive);
                assert_eq!(background_policy(), policy);
            })
            .await;
            assert_eq!(current(), RequestLane::Background);
        })
        .await;
        assert_eq!(background_policy(), BackgroundPolicy::default());
    }
}
