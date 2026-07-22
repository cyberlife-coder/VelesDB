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
//! outstanding. It tracks the count itself (an [`AtomicUsize`]) rather than
//! reaching into a specific implementation's internals, so it works for
//! `LocalSessionManager` today and for any future custom `SessionManager`
//! (e.g. a Redis-backed one) the same way.
//!
//! # Caveat inherited from `rmcp`
//!
//! A session that goes idle past its `SessionConfig::keep_alive` timeout has
//! its worker task exit — but nothing calls [`SessionManager::close_session`]
//! for it (confirmed against rmcp 2.2.0: `LocalSessionManager`'s `sessions`
//! map is touched only by `create_session`, `close_session`, and
//! `restore_session`; there is no reaper). So our counter, like the
//! underlying map, only shrinks on an explicit close (an HTTP DELETE from a
//! well-behaved client) — pure inactivity never frees a slot. In this
//! daemon's actual deployment (a handful of local, cooperating MCP clients)
//! that only matters over very long uptimes; a full fix belongs upstream in
//! `rmcp`, not here.

use std::sync::atomic::{AtomicUsize, Ordering};

use futures::Stream;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
};
use thiserror::Error;

/// Wraps `inner: SM`, refusing new sessions once `max_sessions` are live.
#[derive(Debug)]
pub struct BoundedSessionManager<SM> {
    inner: SM,
    max_sessions: usize,
    live: AtomicUsize,
}

impl<SM> BoundedSessionManager<SM> {
    pub fn new(inner: SM, max_sessions: usize) -> Self {
        Self {
            inner,
            max_sessions,
            live: AtomicUsize::new(0),
        }
    }

    /// Reserve one slot, or refuse if `max_sessions` is already reached.
    /// CAS loop (not load-then-store) so two concurrent callers can't both
    /// observe room for the last slot and overshoot the bound.
    fn try_reserve(&self) -> bool {
        loop {
            let current = self.live.load(Ordering::Acquire);
            if current >= self.max_sessions {
                return false;
            }
            if self
                .live
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Release a slot reserved by [`try_reserve`](Self::try_reserve) that
    /// turned out not to correspond to a real live session (creation
    /// failed, or `restore_session` didn't actually create one). Saturating:
    /// never underflows even if called more than its matching reserve.
    fn release(&self) {
        let _ = self
            .live
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_sub(1))
            });
    }
}

/// Error type for [`BoundedSessionManager`]: either its own bound was hit,
/// or the wrapped `SessionManager` failed on its own terms.
#[derive(Debug, Error)]
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
        if !self.try_reserve() {
            return Err(BoundedSessionManagerError::TooManySessions(
                self.max_sessions,
            ));
        }
        match self.inner.create_session().await {
            Ok(created) => Ok(created),
            Err(e) => {
                self.release();
                Err(e.into())
            }
        }
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
        let result = self.inner.close_session(id).await;
        if result.is_ok() {
            self.release();
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
        if !self.try_reserve() {
            return Err(BoundedSessionManagerError::TooManySessions(
                self.max_sessions,
            ));
        }
        match self.inner.restore_session(id).await {
            Ok(outcome @ RestoreOutcome::Restored(_)) => Ok(outcome),
            // `AlreadyPresent`/`NotSupported` (and any future variant): no
            // new session was actually created, release the reservation.
            Ok(other) => {
                self.release();
                Ok(other)
            }
            Err(e) => {
                self.release();
                Err(e.into())
            }
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
            let mut sessions = self.sessions.lock().expect("lock");
            let before = sessions.len();
            sessions.retain(|existing| existing != id);
            if sessions.len() == before {
                return Err(FakeError(format!("no such session: {id}")));
            }
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
        use std::sync::atomic::AtomicU64;
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
}
