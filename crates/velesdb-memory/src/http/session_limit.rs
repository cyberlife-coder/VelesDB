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
#[path = "session_limit_tests.rs"]
mod tests;
