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
//! `max_sessions` are outstanding, evicts the least-recently-active session
//! that is idle, initialized, and quiet for at least a minimum idle age to
//! admit the new one — refusing only when no live session qualifies (#2289). It tracks the live session ids
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
//! expires (#2289). So at the cap this wrapper reclaims the
//! least-recently-active session instead — provided nothing is using it and
//! it has been quiet for at least the minimum idle age (next section).
//!
//! "In use" cannot be read off a timestamp alone: a session in the middle of
//! a slow tool call or a long-lived SSE stream may have started that activity
//! long ago. `SessionCounters::in_flight` tracks it directly — held by
//! [`ActivityGuard`] for the duration of a request, and for a stream's whole
//! lifetime via [`GuardedStream`]. A session's recency is stamped when its
//! activity ENDS, so a request that ran for ten minutes and finished a second
//! ago counts as a second old, not ten minutes.
//!
//! A session that has been created but not yet initialized is busy too
//! (`SessionCounters::awaiting_initialize`). rmcp calls `create_session`,
//! spawns the session worker, and only then `initialize_session`; a session
//! that looked idle in that gap could be evicted by a concurrent client at
//! the cap, and its own `initialize` would then fail with a `500`. The flag
//! is cleared synchronously, before the first await of `initialize_session`,
//! and in the same step its activity guard marks the session busy until the
//! handshake returns: success, failure, or a request dropped mid-handshake
//! all leave the session evictable afterwards. A session whose
//! `initialize_session` is never called at all (rmcp's restore path can fail
//! between `restore_session` and it) stays unevictable, but not for long:
//! `LocalSessionManager`'s worker gives up after `SessionConfig::init_timeout`
//! (60 s by default) without an `initialize`, and rmcp then closes the
//! session, which frees its slot.
//!
//! # Two locks, and why marking a session busy never awaits
//!
//! Admission (the cap check, the eviction it may trigger, the creation) and
//! close are serialized by an async lock held across the inner manager's
//! awaits, so two admissions can never both take the last slot. The map of
//! live sessions sits behind a separate plain mutex that is never held
//! across an await. Every per-call guard, and the birth flag above, is
//! therefore taken synchronously at the start of the call: a request that is
//! cancelled cannot stop halfway between "looked up" and "marked busy", and a
//! request waiting on an admission in progress cannot leave a session
//! stranded in its busy-at-birth state. Eviction picks its victim under that
//! same plain mutex, so a session is either marked busy before the pick (and
//! skipped) or not at all.
//!
//! # Who can evict whom: the minimum idle age
//!
//! The HTTP transport authenticates no one; it is loopback-only by default,
//! but any local process may send `initialize`. Without a floor, a process
//! repeating `initialize` at the cap would evict every live client that
//! merely had no request and no stream open at that instant — a client
//! thinking between two tool calls included.
//!
//! Eviction therefore only considers sessions idle for at least `min_idle`
//! (`VELESDB_MEMORY_HTTP_EVICT_MIN_IDLE_SECS`, default
//! `DEFAULT_HTTP_EVICT_MIN_IDLE` in the parent module). A client that sends a
//! request more often than that, or keeps its standalone SSE stream open, is
//! never evicted, however hard others push; when every session is busy,
//! initializing, or younger than the floor, the new client is refused exactly
//! as before #2289.
//!
//! The trade-off that remains, stated so nobody has to rediscover it: at the
//! cap, a local process opening sessions CAN evict a live client that has
//! been silent, with no open stream, for longer than `min_idle`. That client
//! gets a `404` on its next request and must re-initialize — and a client
//! that mishandles that `404` can lose the call (#1727). Raising `min_idle`
//! narrows that window but lengthens the time a dead client can lock others
//! out; `keep_alive` bounds it from above, since a session silent that long
//! is retired anyway.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::Stream;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
};
use thiserror::Error;

/// Where [`BoundedSessionManager`] reads "now" when it measures how long a
/// session has been idle. Production uses [`MonotonicClock`]; tests inject a
/// clock they advance by hand, so the minimum idle age is exercised on both
/// sides without a single real sleep.
pub(crate) trait Clock: std::fmt::Debug + Send + Sync + 'static {
    /// Time elapsed since this clock's own fixed origin. Must never go
    /// backward.
    fn elapsed(&self) -> Duration;
}

/// The production [`Clock`]: [`Instant`] measured from construction.
#[derive(Debug)]
pub(crate) struct MonotonicClock(Instant);

impl MonotonicClock {
    pub(crate) fn start() -> Self {
        Self(Instant::now())
    }
}

impl Clock for MonotonicClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Per-session activity bookkeeping used to pick an eviction victim.
///
/// Lives behind an `Arc` rather than directly in the `live` map's value,
/// because releasing `in_flight` happens from [`ActivityGuard`]'s `Drop`,
/// which should not have to take the map's lock at all.
#[derive(Debug)]
struct SessionCounters {
    /// Calls (and open streams) currently being served for this session.
    /// Non-zero means "busy" — never pick this session for eviction.
    in_flight: AtomicU32,
    /// Set from creation until `initialize_session` is entered: a session
    /// still waiting for its own handshake is not idle, it is being born.
    awaiting_initialize: AtomicBool,
    /// Logical tick of the most recent end of activity; orders the LRU pick
    /// strictly, even between stamps the clock cannot tell apart.
    last_tick: AtomicU64,
    /// [`Clock::elapsed`], in nanoseconds, at that same moment; measures the
    /// minimum idle age.
    last_touched_nanos: AtomicU64,
}

/// The shared source of the two stamps a session carries: a strictly
/// increasing tick for LRU order, and the clock for idle age.
#[derive(Debug)]
struct ActivityClock {
    tick: AtomicU64,
    clock: Arc<dyn Clock>,
}

impl ActivityClock {
    fn now_nanos(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn touch(&self, counters: &SessionCounters) {
        counters
            .last_tick
            .store(self.tick.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        counters
            .last_touched_nanos
            .store(self.now_nanos(), Ordering::Relaxed);
    }

    fn new_session(&self) -> Arc<SessionCounters> {
        let counters = SessionCounters {
            in_flight: AtomicU32::new(0),
            awaiting_initialize: AtomicBool::new(true),
            last_tick: AtomicU64::new(0),
            last_touched_nanos: AtomicU64::new(0),
        };
        self.touch(&counters);
        Arc::new(counters)
    }

    /// Nothing in flight, handshake done, and quiet for at least `min_idle`.
    fn is_evictable(&self, counters: &SessionCounters, min_idle: Duration) -> bool {
        if counters.in_flight.load(Ordering::Relaxed) != 0
            || counters.awaiting_initialize.load(Ordering::Relaxed)
        {
            return false;
        }
        let idle_nanos = self
            .now_nanos()
            .saturating_sub(counters.last_touched_nanos.load(Ordering::Relaxed));
        u128::from(idle_nanos) >= min_idle.as_nanos()
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
    clock: Arc<ActivityClock>,
}

impl ActivityGuard {
    /// Must be called while the `live` mutex is held, so no eviction can
    /// choose this session between its lookup and this increment.
    fn begin(counters: Arc<SessionCounters>, clock: Arc<ActivityClock>) -> Self {
        counters.in_flight.fetch_add(1, Ordering::Relaxed);
        Self { counters, clock }
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        // Stamp BEFORE releasing: the instant `in_flight` reaches zero, the
        // session must already read as just-active, never as idle since the
        // activity's start.
        self.clock.touch(&self.counters);
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

/// Wraps `inner: SM`, evicting the least-recently-active evictable session
/// once `max_sessions` are live rather than refusing the new one outright,
/// and refusing only when no live session is evictable.
#[derive(Debug)]
pub struct BoundedSessionManager<SM> {
    inner: SM,
    max_sessions: usize,
    min_idle: Duration,
    /// Serializes changes to the SET of live sessions (admission and close)
    /// across the inner manager's awaits.
    admission: tokio::sync::Mutex<()>,
    /// The live sessions. Never held across an await; see the module docs.
    live: Mutex<HashMap<SessionId, Arc<SessionCounters>>>,
    activity: Arc<ActivityClock>,
}

impl<SM> BoundedSessionManager<SM> {
    pub fn new(inner: SM, max_sessions: usize, min_idle: Duration) -> Self {
        Self::with_clock(
            inner,
            max_sessions,
            min_idle,
            Arc::new(MonotonicClock::start()),
        )
    }

    pub(crate) fn with_clock(
        inner: SM,
        max_sessions: usize,
        min_idle: Duration,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner,
            max_sessions,
            min_idle,
            admission: tokio::sync::Mutex::new(()),
            live: Mutex::new(HashMap::new()),
            activity: Arc::new(ActivityClock {
                tick: AtomicU64::new(0),
                clock,
            }),
        }
    }

    /// The live-session map. Every critical section on it is a few map or
    /// atomic operations that cannot leave it half-updated, so a poisoned
    /// mutex (a panic elsewhere while it was held) is recovered, not
    /// propagated.
    fn live(&self) -> MutexGuard<'_, HashMap<SessionId, Arc<SessionCounters>>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_full(&self) -> bool {
        self.live().len() >= self.max_sessions
    }

    /// Number of sessions currently believed to be alive.
    #[cfg(test)]
    pub(crate) fn live_count(&self) -> usize {
        self.live().len()
    }

    /// Whether `id` is currently tracked as live.
    #[cfg(test)]
    pub(crate) fn is_live(&self, id: &SessionId) -> bool {
        self.live().contains_key(id)
    }

    /// Start (or, for an id this wrapper does not track, skip) an activity
    /// guard for `id`. Synchronous on purpose: the increment happens under the
    /// `live` mutex eviction picks its victim under, with no await before it,
    /// so neither a concurrent eviction nor a cancelled request can separate
    /// the lookup from the increment. Returns `None` for an unknown id so
    /// callers still forward the operation to `inner` unguarded and let it
    /// answer with its own "session not found".
    fn activity_guard(&self, id: &SessionId) -> Option<ActivityGuard> {
        let live = self.live();
        let counters = live.get(id)?;
        Some(ActivityGuard::begin(
            Arc::clone(counters),
            Arc::clone(&self.activity),
        ))
    }
}

/// Error type for [`BoundedSessionManager`]: either its own bound was hit,
/// or the wrapped `SessionManager` failed on its own terms.
#[derive(Debug, Error)]
#[non_exhaustive] // error enum, grows by nature; matching externally requires a wildcard arm
pub enum BoundedSessionManagerError<E> {
    /// `max_sessions` concurrent MCP sessions are live, and none of them can
    /// be evicted: each is busy, still initializing, or active within the
    /// minimum idle age.
    #[error(
        "too many concurrent MCP sessions (max {max_sessions}) and none has been \
         idle for {min_idle_secs} s, so none can be evicted; retry in a moment"
    )]
    TooManySessions {
        max_sessions: usize,
        min_idle_secs: u64,
    },
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
        // The admission lock spans the check, the eviction it may trigger,
        // AND the creation, so two concurrent callers cannot both see room
        // for the last slot. A failed creation records nothing, so there is
        // no reservation left to leak.
        let _admission = self.admission.lock().await;
        if self.is_full() {
            self.evict_one_idle().await?;
        }
        let (id, transport) = self.inner.create_session().await?;
        self.live().insert(id.clone(), self.activity.new_session());
        Ok((id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        // No await before this point: the guard marks the session busy and the
        // birth flag is cleared in one synchronous step, so even a request
        // dropped at its first await leaves the session evictable once the
        // guard is released.
        let guard = self.activity_guard(id);
        if let Some(guard) = &guard {
            guard
                .counters
                .awaiting_initialize
                .store(false, Ordering::Relaxed);
        }
        let result = self.inner.initialize_session(id, message).await;
        drop(guard);
        result.map_err(Into::into)
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        self.inner.has_session(id).await.map_err(Into::into)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        // Removing an id that is not in the map does nothing, so the second
        // close of the same session — the routine case, `DELETE` from the
        // client plus rmcp's own close when the session worker finishes —
        // cannot free a slot that belongs to a still-live session.
        let _admission = self.admission.lock().await;
        let result = self.inner.close_session(id).await;
        if result.is_ok() {
            self.live().remove(id);
        }
        result.map_err(Into::into)
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let guard = self.activity_guard(id);
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
        let _guard = self.activity_guard(id);
        self.inner
            .accept_message(id, message)
            .await
            .map_err(Into::into)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let guard = self.activity_guard(id);
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
        let guard = self.activity_guard(id);
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
        // Unlike `create_session`, whether a slot is needed at all is only
        // known once `inner` answers: `AlreadyPresent` and `NotSupported` (the
        // trait default, and `LocalSessionManager`'s answer) create nothing,
        // so they must evict nothing. Room is made only for a genuine restore
        // — and if none can be made, the restored session is closed again
        // rather than admitted past the cap.
        let _admission = self.admission.lock().await;
        let outcome = self.inner.restore_session(id.clone()).await?;
        if !matches!(outcome, RestoreOutcome::Restored(_)) {
            return Ok(outcome);
        }
        if self.is_full() {
            if let Err(refusal) = self.evict_one_idle().await {
                drop(outcome);
                self.inner.close_session(&id).await?;
                return Err(refusal);
            }
        }
        self.live().insert(id, self.activity.new_session());
        Ok(outcome)
    }
}

impl<SM> BoundedSessionManager<SM>
where
    SM: SessionManager,
{
    /// Evict the least-recently-active evictable session (see
    /// [`ActivityClock::is_evictable`]), making room for the caller's new
    /// one. Errors with [`BoundedSessionManagerError::TooManySessions`] when
    /// no live session is evictable, since then there is nothing safe to
    /// reclaim. Called with the admission lock held.
    ///
    /// The victim stays in `live` until the inner close has succeeded, so a
    /// close that fails, or a caller dropped mid-close, never leaves an open
    /// session untracked.
    async fn evict_one_idle(&self) -> Result<(), BoundedSessionManagerError<SM::Error>> {
        let victim = self
            .live()
            .iter()
            .filter(|(_, counters)| self.activity.is_evictable(counters, self.min_idle))
            .min_by_key(|(_, counters)| counters.last_tick.load(Ordering::Relaxed))
            .map(|(id, _)| id.clone())
            .ok_or(BoundedSessionManagerError::TooManySessions {
                max_sessions: self.max_sessions,
                min_idle_secs: self.min_idle.as_secs(),
            })?;
        self.inner.close_session(&victim).await?;
        self.live().remove(&victim);
        tracing::info!(
            session = %victim,
            max_sessions = self.max_sessions,
            min_idle_secs = self.min_idle.as_secs(),
            "evicted idle MCP session to admit a new one"
        );
        Ok(())
    }
}

#[cfg(test)]
impl<E> BoundedSessionManagerError<E> {
    fn is_too_many_sessions(&self) -> bool {
        matches!(self, Self::TooManySessions { .. })
    }
}

#[cfg(test)]
#[path = "session_limit_tests.rs"]
mod tests;
