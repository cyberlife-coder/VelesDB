//! Caps the number of concurrent MCP sessions a [`SessionManager`] will
//! create, evicting an idle one to make room rather than refusing outright.
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
//! [`BoundedSessionManager`] wraps any [`SessionManager`] and, once
//! `max_sessions` are outstanding, evicts the least-recently-used session
//! with no activity in flight to admit the new one — refusing only when
//! every live session is busy (#2289). It tracks the live session ids
//! itself rather than reaching into a specific implementation's internals,
//! so it works for `LocalSessionManager` today and for any future custom
//! `SessionManager` (e.g. a Redis-backed one) the same way.
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
//! Hence the map of live session ids to their activity counters: a release
//! is matched to the session it belongs to, removing an absent id is a
//! no-op, and the count cannot underflow.
//!
//! # Idle eviction, and why "in use" needs its own signal
//!
//! `SessionConfig::keep_alive` already retires a session after an hour of
//! silence — but a client that dies without `DELETE` (a killed agent, a
//! crashed process, a host restart) leaves its slot occupied for that whole
//! hour, and at the cap every OTHER live client is locked out until it
//! expires (#2289). Waiting out an hour-long timeout to admit a client that
//! is trying to connect right now is not acceptable, so at the cap this
//! wrapper reclaims the least-recently-used session instead — provided
//! nothing is actually using it.
//!
//! "In use" cannot be read off `last_active` alone: a session in the middle
//! of a slow tool call or a long-lived SSE stream may have started that
//! activity long ago, which would make it look like the oldest, most
//! evictable session by timestamp even though it is the busiest one live.
//! [`SessionCounters::in_flight`] tracks that directly — incremented by
//! [`ActivityGuard`] for the duration of a request, and for a stream's whole
//! lifetime via [`GuardedStream`] — so eviction only ever considers sessions
//! with nothing outstanding, and picks the one among those least recently
//! touched.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
};
use thiserror::Error;
use tokio::sync::Mutex;

/// Per-session activity bookkeeping used to pick an eviction victim.
///
/// Lives behind an `Arc` rather than directly in the `live` map's value,
/// because releasing `in_flight` happens from [`ActivityGuard`]'s `Drop`,
/// which cannot `.await` the async [`Mutex`] guarding that map. Cloning the
/// `Arc` while briefly holding the lock lets the guard update the count
/// afterward through a plain atomic, no lock required.
#[derive(Debug, Default)]
struct SessionCounters {
    /// Calls (and open streams) currently being served for this session.
    /// Non-zero means "busy" — never pick this session for eviction.
    in_flight: AtomicU32,
    /// The tick ([`BoundedSessionManager::next_tick`]) at which the most
    /// recent activity on this session began.
    last_active: AtomicU64,
}

impl SessionCounters {
    fn touched_at(tick: u64) -> Self {
        Self {
            in_flight: AtomicU32::new(0),
            last_active: AtomicU64::new(tick),
        }
    }

    fn is_idle(&self) -> bool {
        self.in_flight.load(Ordering::Relaxed) == 0
    }
}

/// Marks a session busy for as long as this guard lives: created before an
/// operation on that session begins, dropped when it ends. For a plain
/// request/response call that is immediately after the inner call returns;
/// for a stream, [`GuardedStream`] holds the guard for the stream's whole
/// lifetime instead, so a session streaming a response is never evicted
/// mid-flight.
struct ActivityGuard {
    counters: Arc<SessionCounters>,
}

impl ActivityGuard {
    fn begin(counters: Arc<SessionCounters>, tick: u64) -> Self {
        counters.last_active.store(tick, Ordering::Relaxed);
        counters.in_flight.fetch_add(1, Ordering::Relaxed);
        Self { counters }
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.counters.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Wraps a per-session stream so its [`ActivityGuard`] is held for the
/// stream's whole lifetime rather than just the `async fn` call that created
/// it — a session with an open SSE stream must count as busy for as long as
/// that stream is being polled, not only for the instant it was opened.
///
/// `inner` is boxed and pinned rather than held as a bare `S` so this struct
/// stays [`Unpin`] regardless of `S`: moving a `Pin<Box<S>>` around only
/// moves the pointer, never the heap-allocated `S` it points at, so polling
/// it needs no `unsafe` pin projection.
struct GuardedStream<S> {
    inner: Pin<Box<S>>,
    _guard: Option<ActivityGuard>,
}

impl<S: Stream> Stream for GuardedStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.as_mut().poll_next(cx)
    }
}

/// Wraps `inner: SM`, evicting the least-recently-used idle session once
/// `max_sessions` are live rather than refusing the new one outright, and
/// refusing only when every live session is busy.
#[derive(Debug)]
pub struct BoundedSessionManager<SM> {
    inner: SM,
    max_sessions: usize,
    live: Mutex<HashMap<SessionId, Arc<SessionCounters>>>,
    /// Monotonic counter handed out by [`Self::next_tick`]; a plain logical
    /// clock rather than a wall-clock timestamp, so LRU ordering never
    /// depends on the runtime's actual timing.
    tick: AtomicU64,
}

impl<SM> BoundedSessionManager<SM> {
    pub fn new(inner: SM, max_sessions: usize) -> Self {
        Self {
            inner,
            max_sessions,
            live: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(0),
        }
    }

    fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::Relaxed)
    }

    /// Number of sessions currently believed to be alive.
    #[cfg(test)]
    pub(crate) async fn live_count(&self) -> usize {
        self.live.lock().await.len()
    }

    /// Whether `id` is currently tracked as live.
    #[cfg(test)]
    pub(crate) async fn is_live(&self, id: &SessionId) -> bool {
        self.live.lock().await.contains_key(id)
    }

    /// Start (or, for an id this wrapper does not track, skip) an activity
    /// guard for `id`, updating its recency and busy count. Returns `None`
    /// for an unknown id so callers still forward the operation to `inner`
    /// unguarded and let it answer with its own "session not found".
    async fn activity_guard(&self, id: &SessionId) -> Option<ActivityGuard> {
        let counters = self.live.lock().await.get(id)?.clone();
        Some(ActivityGuard::begin(counters, self.next_tick()))
    }
}

/// Error type for [`BoundedSessionManager`]: either its own bound was hit,
/// or the wrapped `SessionManager` failed on its own terms.
#[derive(Debug, Error)]
#[non_exhaustive] // error enum, grows by nature; matching externally requires a wildcard arm
pub enum BoundedSessionManagerError<E> {
    /// `max_sessions` concurrent MCP sessions are already live and busy —
    /// none of them idle enough to reclaim.
    #[error(
        "too many concurrent MCP sessions are active (max {0}); wait for one \
         to finish and retry"
    )]
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
        // The lock spans the check, the eviction it may trigger, AND the
        // creation, so two concurrent callers cannot both see room for the
        // last slot. A failed creation records nothing, so there is no
        // reservation left to leak.
        let mut live = self.live.lock().await;
        if live.len() >= self.max_sessions {
            self.evict_one_idle(&mut live).await?;
        }
        let (id, transport) = self.inner.create_session().await?;
        live.insert(
            id.clone(),
            Arc::new(SessionCounters::touched_at(self.next_tick())),
        );
        Ok((id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        let _guard = self.activity_guard(id).await;
        self.inner
            .initialize_session(id, message)
            .await
            .map_err(Into::into)
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        self.inner.has_session(id).await.map_err(Into::into)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        // Removing an id that is not in the map does nothing, so the second
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
        let guard = self.activity_guard(id).await;
        let stream = self.inner.create_stream(id, message).await?;
        Ok(GuardedStream {
            inner: Box::pin(stream),
            _guard: guard,
        })
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        let _guard = self.activity_guard(id).await;
        self.inner
            .accept_message(id, message)
            .await
            .map_err(Into::into)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let guard = self.activity_guard(id).await;
        let stream = self.inner.create_standalone_stream(id).await?;
        Ok(GuardedStream {
            inner: Box::pin(stream),
            _guard: guard,
        })
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let guard = self.activity_guard(id).await;
        let stream = self.inner.resume(id, last_event_id).await?;
        Ok(GuardedStream {
            inner: Box::pin(stream),
            _guard: guard,
        })
    }

    async fn restore_session(
        &self,
        id: SessionId,
    ) -> Result<RestoreOutcome<Self::Transport>, Self::Error> {
        let mut live = self.live.lock().await;
        if live.len() >= self.max_sessions {
            self.evict_one_idle(&mut live).await?;
        }
        match self.inner.restore_session(id.clone()).await {
            // Only a genuine restore adds a live session. `AlreadyPresent` /
            // `NotSupported` (and any future variant) created nothing, so
            // nothing is recorded and no slot is consumed.
            Ok(outcome @ RestoreOutcome::Restored(_)) => {
                live.insert(id, Arc::new(SessionCounters::touched_at(self.next_tick())));
                Ok(outcome)
            }
            Ok(other) => Ok(other),
            Err(e) => Err(e.into()),
        }
    }
}

impl<SM> BoundedSessionManager<SM>
where
    SM: SessionManager,
{
    /// Evict the least-recently-used session with nothing in flight, making
    /// room for the caller's about-to-be-inserted new one. Errors with
    /// [`BoundedSessionManagerError::TooManySessions`] — the same error a
    /// flat refusal used to return — when every live session is busy, since
    /// then there is genuinely nothing safe to reclaim.
    async fn evict_one_idle(
        &self,
        live: &mut HashMap<SessionId, Arc<SessionCounters>>,
    ) -> Result<(), BoundedSessionManagerError<SM::Error>> {
        let victim = live
            .iter()
            .filter(|(_, counters)| counters.is_idle())
            .min_by_key(|(_, counters)| counters.last_active.load(Ordering::Relaxed))
            .map(|(id, _)| id.clone())
            .ok_or(BoundedSessionManagerError::TooManySessions(
                self.max_sessions,
            ))?;
        self.inner.close_session(&victim).await?;
        live.remove(&victim);
        tracing::info!(
            session = %victim,
            max_sessions = self.max_sessions,
            "evicted idle MCP session to admit a new one"
        );
        Ok(())
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
