use std::{future::Future, time::Duration};

use cockpit_core::{CockpitService, InspectionError, SessionChange, SessionSubscription};
use cockpit_protocol::{SessionSnapshotResponse, SessionStreamMessage};

use super::error::OperationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStreamPolicy {
    Gateway,
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incoming {
    Closed,
    Activity,
}

pub trait SessionSink: Send {
    fn send(&mut self, message: SessionStreamMessage) -> impl Future<Output = bool> + Send;
    fn incoming(&mut self) -> impl Future<Output = Incoming> + Send;
    fn cancelled(&self) -> bool;
}

#[derive(Debug)]
pub enum Event {
    Snapshot(Result<SessionSnapshotResponse, InspectionError>),
    Subscribed(Result<(), InspectionError>),
    Change(Option<SessionChange>),
    Delivered(bool),
    BackoffElapsed,
    Client(Incoming),
}

#[derive(Debug)]
pub enum Command {
    Emit(SessionStreamMessage),
    FetchSnapshot { race_client: bool },
    Subscribe(SessionSnapshotResponse),
    AwaitChange,
    Backoff(Duration),
    End,
}

#[derive(Clone, Copy, Debug)]
enum Fetch {
    Initial,
    Boundary,
    Live,
}

#[derive(Debug)]
enum AfterDelivery {
    Subscribe(SessionSnapshotResponse),
    Boundary,
    BoundarySnapshot,
    LiveSnapshot,
    GatewayFailure,
    NativeFailure,
    End,
}

#[derive(Debug)]
enum Phase {
    Fetch(Fetch),
    Sending(AfterDelivery),
    Subscribe,
    Live,
    Backoff,
    End,
}

#[derive(Debug)]
pub struct SessionStream {
    session_id: String,
    policy: SessionStreamPolicy,
    generation: u32,
    sequence: u32,
    retry: u32,
    phase: Phase,
}

impl SessionStream {
    pub fn gateway(session_id: String) -> (Self, Command) {
        let stream = Self {
            session_id,
            policy: SessionStreamPolicy::Gateway,
            generation: 1,
            sequence: 1,
            retry: 0,
            phase: Phase::Fetch(Fetch::Initial),
        };
        (stream, Command::FetchSnapshot { race_client: true })
    }

    pub fn native(session_id: String, initial: SessionSnapshotResponse) -> (Self, Command) {
        let stream = Self {
            session_id,
            policy: SessionStreamPolicy::Native,
            generation: 1,
            sequence: 1,
            retry: 0,
            phase: Phase::Sending(AfterDelivery::Boundary),
        };
        let first = stream.snapshot_message(initial);
        (stream, Command::Emit(first))
    }

    pub fn step(&mut self, event: Event) -> Command {
        let phase = std::mem::replace(&mut self.phase, Phase::End);
        match (phase, event) {
            (Phase::Fetch(fetch), Event::Snapshot(result)) => self.snapshot(fetch, result),
            (Phase::Sending(after), Event::Delivered(true)) => self.delivered(after),
            (Phase::Subscribe, Event::Subscribed(Ok(()))) => {
                // The adapter's successful subscription establishes the live event
                // boundary. Re-read after that boundary so events buffered meanwhile
                // are ordered after an authoritative post-subscription snapshot.
                self.fetch(Fetch::Boundary)
            }
            (Phase::Subscribe, Event::Subscribed(Err(error))) => self.failure(
                true,
                error.code,
                error.message,
                AfterDelivery::GatewayFailure,
            ),
            (Phase::Live, Event::Change(change)) => self.change(change),
            (Phase::Backoff, Event::BackoffElapsed) => self.fetch(Fetch::Initial),
            (phase, Event::Client(Incoming::Activity))
                if self.policy == SessionStreamPolicy::Gateway =>
            {
                match phase {
                    Phase::Fetch(Fetch::Initial) | Phase::Subscribe | Phase::Backoff => {
                        self.fetch(Fetch::Initial)
                    }
                    Phase::Live => self.await_change(),
                    _ => Command::End,
                }
            }
            // Failed sends and client closure end the stream without advancing it.
            _ => Command::End,
        }
    }

    fn snapshot(
        &mut self,
        fetch: Fetch,
        result: Result<SessionSnapshotResponse, InspectionError>,
    ) -> Command {
        let snapshot = match result {
            Ok(snapshot) if snapshot.session_id == self.session_id => snapshot,
            Ok(_) => {
                return self.snapshot_failure(
                    fetch,
                    "session_snapshot_mismatch".to_owned(),
                    "Session snapshot unavailable".to_owned(),
                );
            }
            Err(error) => return self.snapshot_failure(fetch, error.code, error.message),
        };
        let after = match fetch {
            // Only this snapshot is retained for Subscribe after its frame is sent.
            Fetch::Initial => AfterDelivery::Subscribe(snapshot.clone()),
            Fetch::Boundary => AfterDelivery::BoundarySnapshot,
            Fetch::Live => AfterDelivery::LiveSnapshot,
        };
        self.phase = Phase::Sending(after);
        Command::Emit(self.snapshot_message(snapshot))
    }

    fn snapshot_failure(&mut self, fetch: Fetch, code: String, message: String) -> Command {
        let after = match self.policy {
            SessionStreamPolicy::Gateway => AfterDelivery::GatewayFailure,
            SessionStreamPolicy::Native => match fetch {
                Fetch::Live => AfterDelivery::NativeFailure,
                Fetch::Initial | Fetch::Boundary => AfterDelivery::End,
            },
        };
        self.failure(true, code, message, after)
    }

    fn change(&mut self, change: Option<SessionChange>) -> Command {
        let after = match self.policy {
            SessionStreamPolicy::Gateway => AfterDelivery::GatewayFailure,
            SessionStreamPolicy::Native => AfterDelivery::NativeFailure,
        };
        match change {
            Some(SessionChange::Changed) => self.fetch(Fetch::Live),
            Some(SessionChange::Stale { code, message }) => {
                self.failure(true, code, message, after)
            }
            Some(SessionChange::Disconnected { code, message }) => {
                self.failure(false, code, message, after)
            }
            None => {
                let (message, after) = match self.policy {
                    SessionStreamPolicy::Gateway => ("Session stream disconnected", after),
                    SessionStreamPolicy::Native => {
                        ("The session stream disconnected", AfterDelivery::End)
                    }
                };
                self.failure(
                    false,
                    "subscription_closed".to_owned(),
                    message.to_owned(),
                    after,
                )
            }
        }
    }

    fn delivered(&mut self, after: AfterDelivery) -> Command {
        match after {
            AfterDelivery::End => Command::End,
            AfterDelivery::NativeFailure => {
                if self.next_generation() {
                    self.await_change()
                } else {
                    Command::End
                }
            }
            AfterDelivery::GatewayFailure => {
                // A wrapped failure advances once for its frame and once for reconnect.
                if !self.advance() || !self.next_generation() {
                    return Command::End;
                }
                let delay = Duration::from_millis(25 * (1u64 << self.retry.min(5)));
                self.retry = self.retry.saturating_add(1);
                self.phase = Phase::Backoff;
                Command::Backoff(delay)
            }
            after => {
                if !self.advance() {
                    return match (self.policy, after) {
                        (SessionStreamPolicy::Native, AfterDelivery::LiveSnapshot) => self.failure(
                            true,
                            "sequence_overflow".to_owned(),
                            "The stream sequence overflowed".to_owned(),
                            AfterDelivery::End,
                        ),
                        _ => Command::End,
                    };
                }
                match after {
                    AfterDelivery::Subscribe(snapshot) => {
                        self.phase = Phase::Subscribe;
                        Command::Subscribe(snapshot)
                    }
                    AfterDelivery::Boundary => self.fetch(Fetch::Boundary),
                    AfterDelivery::BoundarySnapshot | AfterDelivery::LiveSnapshot => {
                        self.await_change()
                    }
                    _ => Command::End,
                }
            }
        }
    }

    fn advance(&mut self) -> bool {
        if self.sequence == u32::MAX {
            self.next_generation()
        } else {
            self.sequence += 1;
            true
        }
    }

    fn next_generation(&mut self) -> bool {
        let Some(generation) = self.generation.checked_add(1) else {
            return false;
        };
        self.generation = generation;
        self.sequence = 1;
        true
    }

    fn fetch(&mut self, fetch: Fetch) -> Command {
        self.phase = Phase::Fetch(fetch);
        Command::FetchSnapshot {
            race_client: matches!(fetch, Fetch::Initial),
        }
    }

    fn await_change(&mut self) -> Command {
        self.phase = Phase::Live;
        Command::AwaitChange
    }

    fn snapshot_message(&self, snapshot: SessionSnapshotResponse) -> SessionStreamMessage {
        SessionStreamMessage::Snapshot {
            session_id: self.session_id.clone(),
            generation: self.generation,
            sequence: self.sequence,
            snapshot,
        }
    }

    fn failure(
        &mut self,
        stale: bool,
        code: String,
        message: String,
        after: AfterDelivery,
    ) -> Command {
        self.phase = Phase::Sending(after);
        let frame = if stale {
            SessionStreamMessage::Stale {
                session_id: self.session_id.clone(),
                generation: self.generation,
                sequence: self.sequence,
                code,
                message,
            }
        } else {
            SessionStreamMessage::Disconnected {
                session_id: self.session_id.clone(),
                generation: self.generation,
                sequence: self.sequence,
                code,
                message,
            }
        };
        Command::Emit(frame)
    }
}

pub async fn open_native(
    service: &CockpitService,
    session_id: &str,
) -> Result<(SessionSnapshotResponse, SessionSubscription), OperationError> {
    let initial = service.session_snapshot(session_id).await?;
    if initial.session_id != session_id {
        return Err(OperationError::rejected(
            "session_snapshot_mismatch",
            "Session snapshot unavailable",
        ));
    }
    let subscription = service.subscribe_session(session_id, &initial).await?;
    Ok((initial, subscription))
}

pub async fn pump<S: SessionSink>(
    service: CockpitService,
    mut stream: SessionStream,
    first: Command,
    mut subscription: Option<SessionSubscription>,
    sink: &mut S,
) {
    let mut command = first;
    loop {
        let event = match command {
            Command::Emit(message) => Event::Delivered(sink.send(message).await),
            Command::FetchSnapshot { race_client: true } => {
                // Keep the old feed through the backoff; drop it only when restarting.
                subscription = None;
                tokio::select! {
                    result = service.session_snapshot(&stream.session_id) => Event::Snapshot(result),
                    incoming = sink.incoming() => Event::Client(incoming),
                }
            }
            Command::FetchSnapshot { race_client: false } => {
                Event::Snapshot(service.session_snapshot(&stream.session_id).await)
            }
            Command::Subscribe(snapshot) => {
                tokio::select! {
                    result = service.subscribe_session(&stream.session_id, &snapshot) => {
                        match result {
                            Ok(opened) => {
                                subscription = Some(opened);
                                Event::Subscribed(Ok(()))
                            }
                            Err(error) => Event::Subscribed(Err(error)),
                        }
                    }
                    incoming = sink.incoming() => Event::Client(incoming),
                }
            }
            Command::AwaitChange => {
                // Native's existing cancellation flag is checked only at the live loop top.
                if stream.policy == SessionStreamPolicy::Native && sink.cancelled() {
                    return;
                }
                let Some(subscription) = subscription.as_mut() else {
                    return;
                };
                match stream.policy {
                    SessionStreamPolicy::Gateway => tokio::select! {
                        change = subscription.messages.recv() => Event::Change(change),
                        incoming = sink.incoming() => Event::Client(incoming),
                    },
                    SessionStreamPolicy::Native => {
                        Event::Change(subscription.messages.recv().await)
                    }
                }
            }
            Command::Backoff(delay) => tokio::select! {
                _ = tokio::time::sleep(delay) => Event::BackoffElapsed,
                incoming = sink.incoming() => Event::Client(incoming),
            },
            Command::End => return,
        };
        command = stream.step(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SessionSnapshotResponse {
        SessionSnapshotResponse {
            session_id: "session".to_owned(),
            server_instance: "server".to_owned(),
            version: "version".to_owned(),
            protocol: 1,
            focused_space_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            spaces: Vec::new(),
            tabs: Vec::new(),
            panes: Vec::new(),
            agents: Vec::new(),
            herdr_shell: None,
        }
    }

    fn assert_snapshot(command: Command, generation: u32, sequence: u32) {
        assert_eq!(
            emitted(command),
            SessionStreamMessage::Snapshot {
                session_id: "session".to_owned(),
                generation,
                sequence,
                snapshot: snapshot(),
            },
        );
    }

    fn emitted(command: Command) -> SessionStreamMessage {
        let Command::Emit(frame) = command else {
            panic!("Expected frame, got {command:?}");
        };
        frame
    }

    fn assert_fetch(command: Command, race_client: bool) {
        assert!(
            matches!(command, Command::FetchSnapshot { race_client: actual } if actual == race_client)
        );
    }

    fn assert_backoff(command: Command, millis: u64) {
        assert!(
            matches!(command, Command::Backoff(delay) if delay == Duration::from_millis(millis))
        );
    }

    fn gateway_live(stream: &mut SessionStream) {
        assert_snapshot(
            stream.step(Event::Snapshot(Ok(snapshot()))),
            stream.generation,
            stream.sequence,
        );
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::Subscribe(_)
        ));
        assert_fetch(stream.step(Event::Subscribed(Ok(()))), false);
        assert_snapshot(
            stream.step(Event::Snapshot(Ok(snapshot()))),
            stream.generation,
            stream.sequence,
        );
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::AwaitChange
        ));
    }

    fn native_live() -> SessionStream {
        let (mut stream, first) = SessionStream::native("session".to_owned(), snapshot());
        assert_snapshot(first, 1, 1);
        assert_fetch(stream.step(Event::Delivered(true)), false);
        assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 1, 2);
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::AwaitChange
        ));
        stream
    }

    fn stale() -> Event {
        Event::Change(Some(SessionChange::Stale {
            code: "stale_generation".to_owned(),
            message: "stale".to_owned(),
        }))
    }

    fn disconnected() -> Event {
        Event::Change(Some(SessionChange::Disconnected {
            code: "disconnected".to_owned(),
            message: "disconnected".to_owned(),
        }))
    }

    #[test]
    fn gateway_retries_stale_and_disconnected_without_reset_after_success() {
        let (mut stream, first) = SessionStream::gateway("session".to_owned());
        assert_fetch(first, true);
        for (attempt, millis) in [25, 50, 100, 200, 400, 800, 800, 800]
            .into_iter()
            .enumerate()
        {
            gateway_live(&mut stream);
            let failure = if attempt % 2 == 0 {
                stale()
            } else {
                disconnected()
            };
            assert!(matches!(stream.step(failure), Command::Emit(_)));
            assert_backoff(stream.step(Event::Delivered(true)), millis);
            assert_eq!(
                (stream.generation, stream.sequence),
                (attempt as u32 + 2, 1)
            );
            assert_fetch(stream.step(Event::BackoffElapsed), true);
        }
        assert_eq!(stream.retry, 8);
    }

    #[test]
    fn native_failures_continue_on_the_existing_feed() {
        let mut stream = native_live();
        for event in [stale(), disconnected()] {
            assert!(matches!(stream.step(event), Command::Emit(_)));
            assert!(matches!(
                stream.step(Event::Delivered(true)),
                Command::AwaitChange
            ));
            assert_eq!(stream.sequence, 1);
            assert_eq!(stream.retry, 0);
        }
        assert_eq!(stream.generation, 3);
        assert_fetch(
            stream.step(Event::Change(Some(SessionChange::Changed))),
            false,
        );
        assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 3, 1);
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::AwaitChange
        ));
    }

    #[test]
    fn live_snapshot_sequence_wraps_in_both_policies() {
        let (mut gateway, _) = SessionStream::gateway("session".to_owned());
        gateway_live(&mut gateway);
        let native = native_live();
        for mut stream in [gateway, native] {
            stream.generation = 7;
            stream.sequence = u32::MAX;
            assert_fetch(
                stream.step(Event::Change(Some(SessionChange::Changed))),
                false,
            );
            assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 7, u32::MAX);
            assert!(matches!(
                stream.step(Event::Delivered(true)),
                Command::AwaitChange
            ));
            assert_eq!((stream.generation, stream.sequence), (8, 1));
        }
    }

    #[test]
    fn gateway_failure_at_max_sequence_increments_generation_twice() {
        let (mut stream, _) = SessionStream::gateway("session".to_owned());
        gateway_live(&mut stream);
        stream.generation = 9;
        stream.sequence = u32::MAX;
        assert!(matches!(
            stream.step(stale()),
            Command::Emit(SessionStreamMessage::Stale {
                generation: 9,
                sequence: u32::MAX,
                ..
            })
        ));
        assert_backoff(stream.step(Event::Delivered(true)), 25);
        assert_eq!((stream.generation, stream.sequence), (11, 1));
    }

    #[test]
    fn native_failure_at_max_sequence_increments_generation_only_once() {
        let mut stream = native_live();
        stream.generation = 9;
        stream.sequence = u32::MAX;
        assert!(matches!(stream.step(stale()), Command::Emit(_)));
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::AwaitChange
        ));
        assert_eq!((stream.generation, stream.sequence), (10, 1));
    }

    #[test]
    fn gateway_generation_overflow_ends_at_either_failure_advance() {
        for (generation, sequence) in [
            (u32::MAX, 4),
            (u32::MAX, u32::MAX),
            (u32::MAX - 1, u32::MAX),
        ] {
            let (mut stream, _) = SessionStream::gateway("session".to_owned());
            gateway_live(&mut stream);
            stream.generation = generation;
            stream.sequence = sequence;
            assert!(matches!(stream.step(disconnected()), Command::Emit(_)));
            assert!(matches!(stream.step(Event::Delivered(true)), Command::End));
            assert_eq!(stream.retry, 0);
        }
    }

    #[test]
    fn live_snapshot_overflow_emits_native_stale_but_gateway_ends() {
        let (mut gateway, _) = SessionStream::gateway("session".to_owned());
        gateway_live(&mut gateway);
        let native = native_live();
        for mut stream in [gateway, native] {
            stream.generation = u32::MAX;
            stream.sequence = u32::MAX;
            assert_fetch(
                stream.step(Event::Change(Some(SessionChange::Changed))),
                false,
            );
            assert_snapshot(
                stream.step(Event::Snapshot(Ok(snapshot()))),
                u32::MAX,
                u32::MAX,
            );
            let next = stream.step(Event::Delivered(true));
            match stream.policy {
                SessionStreamPolicy::Gateway => assert!(matches!(next, Command::End)),
                SessionStreamPolicy::Native => {
                    assert_eq!(
                        emitted(next),
                        SessionStreamMessage::Stale {
                            session_id: "session".to_owned(),
                            generation: u32::MAX,
                            sequence: u32::MAX,
                            code: "sequence_overflow".to_owned(),
                            message: "The stream sequence overflowed".to_owned(),
                        }
                    );
                    assert!(matches!(stream.step(Event::Delivered(true)), Command::End));
                }
            }
        }
    }

    #[test]
    fn native_failure_generation_overflow_ends_without_another_frame() {
        let mut stream = native_live();
        stream.generation = u32::MAX;
        stream.sequence = u32::MAX;
        assert!(matches!(stream.step(stale()), Command::Emit(_)));
        assert!(matches!(stream.step(Event::Delivered(true)), Command::End));
    }

    #[test]
    fn gateway_activity_restarts_initial_fetch_and_pending_subscription() {
        let (mut stream, first) = SessionStream::gateway("session".to_owned());
        assert_fetch(first, true);
        assert_fetch(stream.step(Event::Client(Incoming::Activity)), true);
        assert_eq!((stream.generation, stream.sequence), (1, 1));
        assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 1, 1);
        let Command::Subscribe(initial) = stream.step(Event::Delivered(true)) else {
            panic!("Expected subscription");
        };
        assert_eq!(initial, snapshot());
        assert_fetch(stream.step(Event::Client(Incoming::Activity)), true);
        assert_eq!((stream.generation, stream.sequence), (1, 2));
        assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 1, 2);
    }

    #[test]
    fn gateway_live_activity_does_not_fetch_or_advance() {
        let (mut stream, _) = SessionStream::gateway("session".to_owned());
        gateway_live(&mut stream);
        assert!(matches!(
            stream.step(Event::Client(Incoming::Activity)),
            Command::AwaitChange
        ));
        assert_eq!((stream.generation, stream.sequence), (1, 3));
    }

    #[test]
    fn gateway_backoff_activity_restarts_without_resetting_retry() {
        let (mut stream, _) = SessionStream::gateway("session".to_owned());
        assert!(matches!(
            stream.step(Event::Snapshot(Err(InspectionError::new(
                "failed", "failed"
            )))),
            Command::Emit(_)
        ));
        assert_backoff(stream.step(Event::Delivered(true)), 25);
        assert_fetch(stream.step(Event::Client(Incoming::Activity)), true);
        assert_eq!(
            (stream.generation, stream.sequence, stream.retry),
            (2, 1, 1)
        );
        gateway_live(&mut stream);
        assert!(matches!(stream.step(stale()), Command::Emit(_)));
        assert_backoff(stream.step(Event::Delivered(true)), 50);
    }

    #[test]
    fn native_boundary_failures_end_regardless_of_delivery() {
        for mismatched in [false, true] {
            for delivered in [false, true] {
                let (mut stream, _) = SessionStream::native("session".to_owned(), snapshot());
                assert_fetch(stream.step(Event::Delivered(true)), false);
                let result = if mismatched {
                    let mut other = snapshot();
                    other.session_id = "other".to_owned();
                    Ok(other)
                } else {
                    Err(InspectionError::new("unavailable", "unavailable"))
                };
                assert!(matches!(
                    stream.step(Event::Snapshot(result)),
                    Command::Emit(SessionStreamMessage::Stale {
                        generation: 1,
                        sequence: 2,
                        ..
                    })
                ));
                assert!(matches!(
                    stream.step(Event::Delivered(delivered)),
                    Command::End
                ));
                assert_eq!((stream.generation, stream.sequence), (1, 2));
            }
        }
    }

    #[test]
    fn native_live_fetch_failures_continue_without_subscribing() {
        for mismatched in [false, true] {
            let mut stream = native_live();
            assert_fetch(
                stream.step(Event::Change(Some(SessionChange::Changed))),
                false,
            );
            let result = if mismatched {
                let mut other = snapshot();
                other.session_id = "other".to_owned();
                Ok(other)
            } else {
                Err(InspectionError::new("failed", "failed"))
            };
            assert!(matches!(
                stream.step(Event::Snapshot(result)),
                Command::Emit(SessionStreamMessage::Stale { .. })
            ));
            assert!(matches!(
                stream.step(Event::Delivered(true)),
                Command::AwaitChange
            ));
            assert_eq!((stream.generation, stream.sequence), (2, 1));
        }
    }

    #[test]
    fn gateway_boundary_failures_retry_instead_of_entering_live() {
        let (mut stream, _) = SessionStream::gateway("session".to_owned());
        assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 1, 1);
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::Subscribe(_)
        ));
        assert_fetch(stream.step(Event::Subscribed(Ok(()))), false);
        assert!(matches!(
            stream.step(Event::Snapshot(Err(InspectionError::new(
                "failed", "failed"
            )))),
            Command::Emit(_)
        ));
        assert_backoff(stream.step(Event::Delivered(true)), 25);
        assert_fetch(stream.step(Event::BackoffElapsed), true);
    }

    #[test]
    fn gateway_subscription_failure_retries() {
        let (mut stream, _) = SessionStream::gateway("session".to_owned());
        assert_snapshot(stream.step(Event::Snapshot(Ok(snapshot()))), 1, 1);
        assert!(matches!(
            stream.step(Event::Delivered(true)),
            Command::Subscribe(_)
        ));
        assert!(matches!(
            stream.step(Event::Subscribed(Err(InspectionError::new(
                "failed", "failed"
            )))),
            Command::Emit(SessionStreamMessage::Stale {
                generation: 1,
                sequence: 2,
                ..
            })
        ));
        assert_backoff(stream.step(Event::Delivered(true)), 25);
    }

    #[test]
    fn closed_feed_retries_gateway_but_ends_native_after_its_frame() {
        let (mut gateway, _) = SessionStream::gateway("session".to_owned());
        gateway_live(&mut gateway);
        let native = native_live();
        for mut stream in [gateway, native] {
            assert!(matches!(
                stream.step(Event::Change(None)),
                Command::Emit(SessionStreamMessage::Disconnected {
                    generation: 1,
                    sequence: 3,
                    ..
                })
            ));
            let next = stream.step(Event::Delivered(true));
            match stream.policy {
                SessionStreamPolicy::Gateway => assert_backoff(next, 25),
                SessionStreamPolicy::Native => assert!(matches!(next, Command::End)),
            }
        }
    }

    #[test]
    fn failed_send_ends_without_advancing_snapshot_or_failure() {
        let (mut gateway, _) = SessionStream::gateway("session".to_owned());
        gateway_live(&mut gateway);
        let native = native_live();
        for mut stream in [gateway, native] {
            let stamp = (stream.generation, stream.sequence);
            assert!(matches!(stream.step(stale()), Command::Emit(_)));
            assert!(matches!(stream.step(Event::Delivered(false)), Command::End));
            assert_eq!((stream.generation, stream.sequence), stamp);
        }
        let (mut stream, _) = SessionStream::native("session".to_owned(), snapshot());
        assert!(matches!(stream.step(Event::Delivered(false)), Command::End));
        assert_eq!((stream.generation, stream.sequence), (1, 1));
    }

    #[test]
    fn client_closure_ends_each_gateway_racing_phase() {
        for phase in [
            Phase::Fetch(Fetch::Initial),
            Phase::Subscribe,
            Phase::Live,
            Phase::Backoff,
        ] {
            let (mut stream, _) = SessionStream::gateway("session".to_owned());
            stream.phase = phase;
            assert!(matches!(
                stream.step(Event::Client(Incoming::Closed)),
                Command::End
            ));
            assert_eq!((stream.generation, stream.sequence), (1, 1));
        }
    }

    #[test]
    fn native_initial_and_boundary_snapshot_overflow_end_silently() {
        for after in [AfterDelivery::Boundary, AfterDelivery::BoundarySnapshot] {
            let (mut stream, _) = SessionStream::native("session".to_owned(), snapshot());
            stream.generation = u32::MAX;
            stream.sequence = u32::MAX;
            stream.phase = Phase::Sending(after);
            assert!(matches!(stream.step(Event::Delivered(true)), Command::End));
        }
    }

    #[test]
    fn gateway_retry_saturates_while_delay_remains_capped() {
        let (mut stream, _) = SessionStream::gateway("session".to_owned());
        stream.retry = u32::MAX;
        assert!(matches!(
            stream.step(Event::Snapshot(Err(InspectionError::new(
                "failed", "failed"
            )))),
            Command::Emit(_)
        ));
        assert_backoff(stream.step(Event::Delivered(true)), 800);
        assert_eq!(stream.retry, u32::MAX);
    }
}
