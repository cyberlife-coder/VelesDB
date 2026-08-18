use super::*;
use rmcp::transport::Transport;
use rmcp::RoleServer;
use std::sync::Mutex;

/// `SessionManager::Transport` must implement `Transport<RoleServer>`,
/// which rules out a bare `()` — this is the smallest thing that
/// qualifies, and every method is unreachable because these tests only
/// exercise `BoundedSessionManager`'s bookkeeping, never the transport.
#[derive(Debug)]
struct FakeTransport;

impl Transport<RoleServer> for FakeTransport {
    type Error = FakeError;

    fn send(
        &mut self,
        _item: ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        std::future::ready(Err(FakeError("FakeTransport::send unreachable".into())))
    }

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<ClientJsonRpcMessage>> + Send {
        std::future::ready(None)
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(Err(FakeError("FakeTransport::close unreachable".into())))
    }
}

/// A tiny in-memory `SessionManager` fake: just enough surface to drive
/// `BoundedSessionManager`'s own logic without pulling in
/// `LocalSessionManager`'s full worker/channel machinery. Each "session"
/// is nothing but an id in a `Vec`.
#[derive(Debug, Default)]
struct FakeSessionManager {
    sessions: Mutex<Vec<SessionId>>,
}

#[derive(Debug, Error)]
#[error("fake session manager error: {0}")]
struct FakeError(String);

impl SessionManager for FakeSessionManager {
    type Error = FakeError;
    type Transport = FakeTransport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        let id: SessionId = format!("fake-{}", uuid_like()).into();
        self.sessions.lock().expect("lock").push(id.clone());
        Ok((id, FakeTransport))
    }

    async fn initialize_session(
        &self,
        _id: &SessionId,
        _message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        unimplemented!("not exercised by these tests")
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        Ok(self.sessions.lock().expect("lock").contains(id))
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        // Modelled on `LocalSessionManager::close_session`, which answers
        // `Ok` whether or not the id was there. That idempotence is not
        // incidental — it is exactly what makes a close-counting wrapper
        // unable to tell a second close from a first, so a fake that
        // errored here would hide the defect these tests exist to pin.
        let mut sessions = self.sessions.lock().expect("lock");
        sessions.retain(|existing| existing != id);
        Ok(())
    }

    async fn create_stream(
        &self,
        _id: &SessionId,
        _message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        Ok(futures::stream::empty())
    }

    async fn accept_message(
        &self,
        _id: &SessionId,
        _message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn create_standalone_stream(
        &self,
        _id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        Ok(futures::stream::empty())
    }

    async fn resume(
        &self,
        _id: &SessionId,
        _last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        Ok(futures::stream::empty())
    }
}

fn uuid_like() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[tokio::test]
async fn create_session_succeeds_under_the_limit() {
    let manager = BoundedSessionManager::new(FakeSessionManager::default(), 2);
    assert!(manager.create_session().await.is_ok());
    assert!(manager.create_session().await.is_ok());
}

#[tokio::test]
async fn create_session_refuses_past_the_limit() {
    let manager = BoundedSessionManager::new(FakeSessionManager::default(), 2);
    manager.create_session().await.expect("first session");
    manager.create_session().await.expect("second session");

    let err = manager
        .create_session()
        .await
        .expect_err("third session must be refused");
    assert!(err.is_too_many_sessions());
}

#[tokio::test]
async fn closing_a_session_frees_a_slot_for_a_new_one() {
    let manager = BoundedSessionManager::new(FakeSessionManager::default(), 1);
    let (id, _transport) = manager.create_session().await.expect("first session");
    manager
        .create_session()
        .await
        .expect_err("second session must be refused while the first is live");

    manager
        .close_session(&id)
        .await
        .expect("close first session");
    assert!(
        manager.create_session().await.is_ok(),
        "closing the first session must free its slot"
    );
}

#[tokio::test]
async fn a_failed_create_session_does_not_leak_a_reserved_slot() {
    // FakeSessionManager::create_session never fails on its own, so
    // drive this through a manager wrapping ONE that always fails, and
    // confirm the reservation `try_reserve` took is released — i.e. the
    // bound isn't silently consumed by inner failures.
    #[derive(Debug, Default)]
    struct AlwaysFails;

    impl SessionManager for AlwaysFails {
        type Error = FakeError;
        type Transport = FakeTransport;

        async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
            Err(FakeError("always fails".into()))
        }

        async fn initialize_session(
            &self,
            _id: &SessionId,
            _message: ClientJsonRpcMessage,
        ) -> Result<ServerJsonRpcMessage, Self::Error> {
            unimplemented!()
        }

        async fn has_session(&self, _id: &SessionId) -> Result<bool, Self::Error> {
            Ok(false)
        }

        async fn close_session(&self, _id: &SessionId) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn create_stream(
            &self,
            _id: &SessionId,
            _message: ClientJsonRpcMessage,
        ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
        {
            Ok(futures::stream::empty())
        }

        async fn accept_message(
            &self,
            _id: &SessionId,
            _message: ClientJsonRpcMessage,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn create_standalone_stream(
            &self,
            _id: &SessionId,
        ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
        {
            Ok(futures::stream::empty())
        }

        async fn resume(
            &self,
            _id: &SessionId,
            _last_event_id: String,
        ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
        {
            Ok(futures::stream::empty())
        }
    }

    let manager = BoundedSessionManager::new(AlwaysFails, 1);
    manager
        .create_session()
        .await
        .expect_err("inner always fails");
    // If the reservation had leaked, this would also be refused with
    // `TooManySessions` instead of the inner's own error.
    let err = manager
        .create_session()
        .await
        .expect_err("inner still always fails");
    assert!(
        !err.is_too_many_sessions(),
        "a failed create must not leak its reservation: {err}"
    );
}

#[tokio::test]
async fn closing_the_same_session_twice_frees_exactly_one_slot() {
    // The routine double close: the client's DELETE, then rmcp's own
    // close when the session worker finishes. An anonymous counter
    // decremented twice here would drift below reality.
    let manager = BoundedSessionManager::new(FakeSessionManager::default(), 2);
    let (a, _ta) = manager.create_session().await.expect("session A");
    let (_b, _tb) = manager.create_session().await.expect("session B");
    assert_eq!(manager.live_count().await, 2);

    manager.close_session(&a).await.expect("first close");
    manager.close_session(&a).await.expect("second close");

    assert_eq!(
        manager.live_count().await,
        1,
        "closing ONE session twice must still free exactly one slot"
    );
    manager
        .create_session()
        .await
        .expect("the freed slot must be reusable");
    manager
        .create_session()
        .await
        .expect_err("but only ONE slot was freed, so the next must be refused");
}

#[tokio::test]
async fn closing_an_unknown_session_frees_nothing() {
    let manager = BoundedSessionManager::new(FakeSessionManager::default(), 1);
    let (_a, _ta) = manager.create_session().await.expect("session A");

    let stranger: SessionId = "never-created".to_string().into();
    manager
        .close_session(&stranger)
        .await
        .expect("closing an unknown id is a no-op, not an error");

    assert_eq!(manager.live_count().await, 1);
    manager
        .create_session()
        .await
        .expect_err("an unknown id must not free the live session's slot");
}

#[tokio::test]
async fn many_create_then_close_cycles_never_exhaust_the_bound() {
    // The guarantee that matters in production: a daemon cycling sessions
    // far more times than `max_sessions` must never lock itself out.
    let manager = BoundedSessionManager::new(FakeSessionManager::default(), 2);
    for cycle in 0..64 {
        let (id, _t) = manager
            .create_session()
            .await
            .unwrap_or_else(|_| panic!("cycle {cycle} must still be able to open a session"));
        manager.close_session(&id).await.expect("close");
    }
    assert_eq!(manager.live_count().await, 0);
}
