//! Caps the number of concurrent MCP sessions a [`SessionManager`] will
//! create.
//!
//! `rmcp`'s `LocalSessionManager` (the in-memory session store [`router`](
//! super::router) uses) has no such cap built in: `create_session` always
//! succeeds and inserts into its `sessions` map, so a client that opens
//! sessions without ever closing them — malicious, or just buggy — can grow
//! that map without bound. Each session spawns its own worker task plus two
//! bounded mpsc channels (`SessionConfig::channel_capacity`, 16 by default)
//! — individually small, but with no ceiling on the session COUNT the
//! aggregate is unbounded, exactly the shape of resource exhaustion this
//! module exists to close off.
//!
//! [`BoundedSessionManager`] wraps any [`SessionManager`] and refuses
//! `create_session`/`restore_session` once `max_sessions` sessions are
//! outstanding. It tracks the live session ids itself rather than reaching
//! into a specific implementation's internals, so it works for
//! `LocalSessionManager` today and for any future custom `SessionManager`
//! (e.g. a Redis-backed one) the same way.
//!
//! # Session lifetime, and why the bound tracks IDS rather than a count
//!
//! An earlier version of this comment claimed that a session going idle past
//! `SessionConfig::keep_alive` never frees its slot, because nothing calls
//! [`SessionManager::close_session`] for it. **That is wrong**, and it is
//! worth stating plainly because it sent one investigation down the wrong
//! path: rmcp's `StreamableHttpService` spawns a task per session that awaits
//! the service and then calls `close_session` (rmcp 2.2.0,
//! `streamable_http_server/tower.rs`), so an idle-expired session IS closed
//! and its slot IS returned. `tests/http_transport.rs` pins that down.
//!
//! The real hazard is the opposite one. A session is routinely closed
//! **twice**: once by the `DELETE` a well-behaved client sends, and once by
//! that per-session task when the service finishes. `LocalSessionManager`'s
//! `close_session` is idempotent and answers `Ok` either way, so a wrapper
//! counting closes cannot tell the second from a first — and an anonymous
//! counter decremented twice for one session drifts BELOW reality, letting
//! more than `max_sessions` run at once and quietly weakening the bound this
//! module exists to enforce.
//!
//! Hence the set of live session ids: a release is matched to the session it
//! belongs to, removing an absent id is a no-op, and the count cannot
//! underflow.

use std::collections::HashSet;

use futures::Stream;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
};
use thiserror::Error;
use tokio::sync::Mutex;

/// Wraps `inner: SM`, refusing new sessions once `max_sessions` are live.
///
/// The bound is enforced against the SET of session ids this wrapper has
/// created and not yet seen closed, rather than against a bare count. That
/// distinction is the whole point: a count is anonymous, so nothing ties a
/// decrement to the session it belongs to, and one session closed twice
/// decrements twice. Sessions ARE closed twice in normal operation — a
/// well-behaved client sends `DELETE`, and rmcp's own session worker calls
/// `close_session` again when the service finishes — so an anonymous count
/// drifts below reality and lets more than `max_sessions` run at once.
///
/// With a set, releasing is idempotent by construction (removing an id that
/// is not there does nothing), the count can never underflow, and
/// `live.len()` is exactly the set of sessions believed to be alive.
#[derive(Debug)]
pub struct BoundedSessionManager<SM> {
    inner: SM,
    max_sessions: usize,
    live: Mutex<HashSet<SessionId>>,
}

impl<SM> BoundedSessionManager<SM> {
    pub fn new(inner: SM, max_sessions: usize) -> Self {
        Self {
            inner,
            max_sessions,
            live: Mutex::new(HashSet::new()),
        }
    }

    /// Number of sessions currently believed to be alive.
    #[cfg(test)]
    pub(crate) async fn live_count(&self) -> usize {
        self.live.lock().await.len()
    }
}

/// Error type for [`BoundedSessionManager`]: either its own bound was hit,
/// or the wrapped `SessionManager` failed on its own terms.
#[derive(Debug, Error)]
#[non_exhaustive] // error enum, grows by nature; matching externally requires a wildcard arm
pub enum BoundedSessionManagerError<E> {
    /// `max_sessions` concurrent MCP sessions are already live.
    #[error("too many concurrent MCP sessions (max {0}); close an existing one and retry")]
    TooManySessions(usize),
    /// The wrapped session manager itself failed.
    #[error(transparent)]
    Inner(#[from] E),
}

impl<SM> SessionManager for BoundedSessionManager<SM>
where
    SM: SessionManager,
{
    type Error = BoundedSessionManagerError<SM::Error>;
    type Transport = SM::Transport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        // The lock spans the check AND the creation, so two concurrent
        // callers cannot both see room for the last slot. A failed creation
        // records nothing, so there is no reservation left to leak.
        let mut live = self.live.lock().await;
        if live.len() >= self.max_sessions {
            return Err(BoundedSessionManagerError::TooManySessions(
                self.max_sessions,
            ));
        }
        let (id, transport) = self.inner.create_session().await?;
        live.insert(id.clone());
        Ok((id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        self.inner
            .initialize_session(id, message)
            .await
            .map_err(Into::into)
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        self.inner.has_session(id).await.map_err(Into::into)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        // Removing an id that is not in the set does nothing, so the second
        // close of the same session — the routine case, `DELETE` from the
        // client plus rmcp's own close when the session worker finishes —
        // cannot free a slot that belongs to a still-live session.
        let mut live = self.live.lock().await;
        let result = self.inner.close_session(id).await;
        if result.is_ok() {
            live.remove(id);
        }
        result.map_err(Into::into)
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.inner
            .create_stream(id, message)
            .await
            .map_err(Into::into)
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        self.inner
            .accept_message(id, message)
            .await
            .map_err(Into::into)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.inner
            .create_standalone_stream(id)
            .await
            .map_err(Into::into)
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.inner
            .resume(id, last_event_id)
            .await
            .map_err(Into::into)
    }

    async fn restore_session(
        &self,
        id: SessionId,
    ) -> Result<RestoreOutcome<Self::Transport>, Self::Error> {
        let mut live = self.live.lock().await;
        if live.len() >= self.max_sessions {
            return Err(BoundedSessionManagerError::TooManySessions(
                self.max_sessions,
            ));
        }
        match self.inner.restore_session(id.clone()).await {
            // Only a genuine restore adds a live session. `AlreadyPresent` /
            // `NotSupported` (and any future variant) created nothing, so
            // nothing is recorded and no slot is consumed.
            Ok(outcome @ RestoreOutcome::Restored(_)) => {
                live.insert(id);
                Ok(outcome)
            }
            Ok(other) => Ok(other),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
impl<E> BoundedSessionManagerError<E> {
    fn is_too_many_sessions(&self) -> bool {
        matches!(self, Self::TooManySessions(_))
    }
}

#[cfg(test)]
mod tests {
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
}
