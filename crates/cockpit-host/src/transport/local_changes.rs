//! Credit-limited transport for local filesystem invalidations.
//!
//! The lease owns the observer and is dropped on every exit path. A cursor is
//! captured before sending each event, so changes during the consumer's read
//! remain visible after its acknowledgement.
use std::{future::Future, sync::{Arc, LazyLock}, time::Duration};

use cockpit_core::local_changes::LocalChangeSubscription;
use cockpit_protocol::local_changes::{LocalChangeAck, LocalChangeEvent, LocalChangeKind, LocalChangeReason, MAX_SAFE_SEQUENCE};

const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared admission across browser and native streams; the owned permit
/// remains held until the transport has released its observer.
pub fn admit() -> Option<tokio::sync::OwnedSemaphorePermit> {
    static LIMIT: LazyLock<Arc<tokio::sync::Semaphore>> =
        LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(256)));
    LIMIT.clone().try_acquire_owned().ok()
}

pub trait LocalChangeSink: Send {
    fn send(&mut self, event: LocalChangeEvent) -> impl Future<Output = bool> + Send;
    /// None means closed, malformed, or otherwise invalid input.
    fn incoming(&mut self) -> impl Future<Output = Option<LocalChangeAck>> + Send;
}

async fn wait_for_ack<S: LocalChangeSink>(
    sink: &mut S, stream_id: &str, sequence: u64, last_acked: u64,
) -> bool {
    let deadline = tokio::time::Instant::now() + ACK_TIMEOUT;
    loop {
        let ack = match tokio::time::timeout_at(deadline, sink.incoming()).await {
            Ok(Some(ack)) => ack,
            _ => return false,
        };
        if ack.validate().is_err() || ack.stream_id != stream_id { return false; }
        if ack.sequence == last_acked && last_acked != 0 { continue; }
        return ack.sequence == sequence;
    }
}

/// Drive one immutable, already-authorized scope. The asynchronous guard pins
/// session-bound authority independently of the filesystem identity guard.
pub async fn pump<S: LocalChangeSink>(subscription: LocalChangeSubscription, stream_id: String, sink: &mut S)
{
    let mut receiver = subscription.receiver();
    let mut event_cursor = subscription.checkpoint();
    let mut sequence = 1u64;
    let mut kind = LocalChangeKind::Ready;
    let mut topics = Vec::new();
    let mut reason = None;
    let mut last_acked = 0;
    if event_cursor.unavailable.is_some() {
        kind = LocalChangeKind::Unavailable;
        reason = Some(LocalChangeReason { code: "watch_unavailable".into(), message: "Automatic updates are unavailable".into() });
    }

    loop {
        // Keep the exact checkpoint captured with the event's topics. A
        // later checkpoint could swallow a concurrent mutation.
        if kind != LocalChangeKind::Unavailable
            && subscription.reauthorize().await.is_err()
        {
            kind = LocalChangeKind::Unavailable;
            topics.clear();
            reason = Some(LocalChangeReason { code: "authorization_lost".into(), message: "Automatic updates are unavailable".into() });
        }
        let event = LocalChangeEvent { stream_id: stream_id.clone(), sequence, kind, topics: std::mem::take(&mut topics), reason: reason.take() };
        if event.validate().is_err() || !matches!(tokio::time::timeout(SEND_TIMEOUT, sink.send(event)).await, Ok(true)) {
            return;
        }
        if !wait_for_ack(sink, &stream_id, sequence, last_acked).await { return; }
        if kind == LocalChangeKind::Unavailable { return; }
        last_acked = sequence;
        if sequence == MAX_SAFE_SEQUENCE { return; }
        sequence += 1;
        let cursor = event_cursor;

        loop {
            let mut changes = subscription.take_since(&cursor);
            if changes.unavailable.is_some() {
                kind = LocalChangeKind::Unavailable;
                reason = Some(LocalChangeReason { code: "watch_unavailable".into(), message: "Automatic updates are unavailable".into() });
                event_cursor = changes;
                break;
            }
            if !changes.topics.is_empty() {
                kind = LocalChangeKind::Changed;
                topics = std::mem::take(&mut changes.topics);
                event_cursor = changes;
                break;
            }
            tokio::select! {
                result = receiver.changed() => if result.is_err() { return; },
                ack = sink.incoming() => match ack {
                    Some(ack) if ack.validate().is_ok() && ack.stream_id == stream_id && ack.sequence == last_acked => {},
                    _ => return,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    struct Sink(mpsc::Receiver<LocalChangeAck>);
    impl LocalChangeSink for Sink {
        async fn send(&mut self, _: LocalChangeEvent) -> bool { true }
        async fn incoming(&mut self) -> Option<LocalChangeAck> { self.0.recv().await }
    }
    fn ack(id: &str, sequence: u64) -> LocalChangeAck {
        LocalChangeAck { stream_id: id.into(), sequence }
    }

    #[tokio::test]
    async fn only_current_credit_is_accepted() {
        let (tx, rx) = mpsc::channel(2);
        tx.send(ack("one", 1)).await.unwrap();
        tx.send(ack("one", 2)).await.unwrap();
        assert!(wait_for_ack(&mut Sink(rx), "one", 2, 1).await);
        let (tx, rx) = mpsc::channel(1);
        tx.send(ack("foreign", 2)).await.unwrap();
        assert!(!wait_for_ack(&mut Sink(rx), "one", 2, 1).await);
        let (tx, rx) = mpsc::channel(1);
        tx.send(ack("one", 3)).await.unwrap();
        assert!(!wait_for_ack(&mut Sink(rx), "one", 2, 1).await);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_or_closed_reader_releases_credit() {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(async move { wait_for_ack(&mut Sink(rx), "one", 1, 0).await });
        tokio::task::yield_now().await;
        tokio::time::advance(ACK_TIMEOUT).await;
        assert!(!task.await.unwrap());
        drop(tx);
        let (tx, rx) = mpsc::channel(1);
        drop(tx);
        assert!(!wait_for_ack(&mut Sink(rx), "one", 1, 0).await);
    }

    #[tokio::test(start_paused = true)]
    async fn duplicate_credit_does_not_extend_deadline() {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(async move { wait_for_ack(&mut Sink(rx), "one", 2, 1).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(29)).await;
        tx.send(ack("one", 1)).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!task.await.unwrap());
    }
}
