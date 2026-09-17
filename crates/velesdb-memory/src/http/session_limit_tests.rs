use super::*;
use futures::FutureExt;
use rmcp::model::{
    ClientNotification, EmptyResult, InitializedNotification, NumberOrString, ServerResult,
};
use rmcp::transport::Transport;
use rmcp::RoleServer;
use std::sync::Mutex;

/// The eviction floor every test manager is built with. Tests move the fake
/// clock across it explicitly, so which side of it a session sits on is
/// always stated, never inferred from how fast the test happened to run.
const TEST_MIN_IDLE: Duration = Duration::from_secs(60);

/// The handshake deadline every test manager is built with — well past any
/// single advance of [`TEST_MIN_IDLE`] the other tests make, so only the test
/// about abandoned handshakes ever reaches it.
const TEST_INIT_TIMEOUT: Duration = Duration::from_secs(600);

/// A `ClientJsonRpcMessage` value for tests driving `create_stream`/`resume`:
/// its content is irrelevant here — `FakeSessionManager` ignores it — only
/// its type matters to satisfy the signature.
fn dummy_message() -> ClientJsonRpcMessage {
    ClientJsonRpcMessage::notification(ClientNotification::InitializedNotification(
        InitializedNotification::default(),
    ))
}

/// A [`Clock`] that moves only when a test says so.
#[derive(Debug, Default)]
struct FakeClock {
    nanos: AtomicU64,
}

impl FakeClock {
    fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_nanos()).expect("test durations fit in u64 nanoseconds");
        self.nanos.fetch_add(by, Ordering::Relaxed);
    }
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.nanos.load(Ordering::Relaxed))
    }
}

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
    /// `restore_session` answers `Restored` (and records the id) when set,
    /// the trait's default `NotSupported` otherwise.
    restores: bool,
    /// How `initialize_session` answers.
    initialize: FakeInitialize,
    /// `create_session` fails when set.
    fails_create: bool,
    /// When set, `close_session` waits for a permit before closing, so a test
    /// can act while an eviction's close is still in progress.
    close_gate: Option<Arc<tokio::sync::Semaphore>>,
}

/// How [`FakeSessionManager::initialize_session`] answers.
#[derive(Debug, Default, Clone, Copy)]
enum FakeInitialize {
    #[default]
    Succeeds,
    Fails,
    /// Never completes: stands for a handshake still in progress.
    Hangs,
}

#[derive(Debug, Error)]
#[error("fake session manager error: {0}")]
struct FakeError(String);

impl SessionManager for FakeSessionManager {
    type Error = FakeError;
    type Transport = FakeTransport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        if self.fails_create {
            return Err(FakeError("create refused".into()));
        }
        let id: SessionId = format!("fake-{}", uuid_like()).into();
        self.sessions.lock().expect("lock").push(id.clone());
        Ok((id, FakeTransport))
    }

    async fn initialize_session(
        &self,
        _id: &SessionId,
        _message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        match self.initialize {
            FakeInitialize::Succeeds => Ok(ServerJsonRpcMessage::response(
                ServerResult::EmptyResult(EmptyResult {}),
                NumberOrString::Number(0),
            )),
            FakeInitialize::Fails => Err(FakeError("initialize refused".into())),
            FakeInitialize::Hangs => std::future::pending().await,
        }
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
        if let Some(gate) = &self.close_gate {
            gate.acquire()
                .await
                .expect("close gate is never closed")
                .forget();
        }
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

    async fn restore_session(
        &self,
        id: SessionId,
    ) -> Result<RestoreOutcome<Self::Transport>, Self::Error> {
        if !self.restores {
            return Ok(RestoreOutcome::NotSupported);
        }
        self.sessions.lock().expect("lock").push(id);
        Ok(RestoreOutcome::Restored(FakeTransport))
    }
}

fn uuid_like() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

type TestManager = BoundedSessionManager<FakeSessionManager>;

/// A manager over `inner` with a cap of `max_sessions`, [`TEST_MIN_IDLE`] as
/// its eviction floor, and the fake clock driving it.
fn bounded_over(inner: FakeSessionManager, max_sessions: usize) -> (TestManager, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock::default());
    let manager = BoundedSessionManager::with_clock(
        inner,
        max_sessions,
        EvictionPolicy {
            min_idle: TEST_MIN_IDLE,
            init_timeout: Some(TEST_INIT_TIMEOUT),
        },
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    (manager, clock)
}

fn bounded(max_sessions: usize) -> (TestManager, Arc<FakeClock>) {
    bounded_over(FakeSessionManager::default(), max_sessions)
}

/// Create a session AND complete its handshake, the way rmcp always follows
/// one with the other; only such a session can ever be evicted.
async fn open_initialized(manager: &TestManager) -> SessionId {
    let (id, _transport) = manager.create_session().await.expect("create session");
    manager
        .initialize_session(&id, dummy_message())
        .await
        .expect("initialize session");
    id
}

#[tokio::test]
async fn create_session_succeeds_under_the_limit() {
    let (manager, _clock) = bounded(2);
    assert!(manager.create_session().await.is_ok());
    assert!(manager.create_session().await.is_ok());
}

/// At the cap, a session with nothing in flight is evicted rather than the
/// new one being refused — the whole point of #2289.
#[tokio::test]
async fn create_session_evicts_an_idle_session_past_the_limit() {
    let (manager, clock) = bounded(2);
    let a = open_initialized(&manager).await;
    open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    manager
        .create_session()
        .await
        .expect("a third session must be admitted by evicting an idle one");
    assert_eq!(
        manager.live_count(),
        2,
        "the cap itself must still hold — eviction makes room, it doesn't lift the ceiling"
    );
    assert!(
        !manager.is_live(&a),
        "the least-recently-active idle session (the first) must be the one evicted"
    );
}

/// The positive control for the eviction test above: refusal still happens
/// once every live session is busy and there is nothing left to reclaim. An
/// open stream is what marks a session busy (see `GuardedStream`); it is
/// dropped, not consumed, so `futures::stream::empty` from
/// `FakeSessionManager` never gets a chance to end it on its own.
#[tokio::test]
async fn create_session_refuses_past_the_limit_when_every_session_is_busy() {
    let (manager, clock) = bounded(2);
    let a = open_initialized(&manager).await;
    let b = open_initialized(&manager).await;
    let stream_a = manager
        .create_stream(&a, dummy_message())
        .await
        .expect("stream on session A");
    let stream_b = manager
        .create_stream(&b, dummy_message())
        .await
        .expect("stream on session B");
    clock.advance(TEST_MIN_IDLE);

    let err = manager
        .create_session()
        .await
        .expect_err("a third session must be refused while both live ones are busy");
    assert!(err.is_too_many_sessions());

    drop(stream_a);
    drop(stream_b);
}

/// Among several idle candidates, eviction always picks the one least
/// recently active, not just any of them.
#[tokio::test]
async fn eviction_picks_the_least_recently_used_idle_session() {
    let (manager, clock) = bounded(2);
    let a = open_initialized(&manager).await;
    let b = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    manager
        .create_session()
        .await
        .expect("third session admitted by evicting the oldest idle one");
    assert!(!manager.is_live(&a), "A is oldest, so A is evicted");
    assert!(manager.is_live(&b), "B is newer, so B survives");
}

/// Recency follows ACTIVITY, not creation order: A is created first but used
/// again afterwards, B is created later and never used, so B is the least
/// recently active and must go. Without the stamp an activity leaves behind,
/// A would still look oldest and be evicted instead.
#[tokio::test]
async fn activity_after_creation_makes_a_session_more_recent() {
    let (manager, clock) = bounded(2);
    let a = open_initialized(&manager).await;
    let b = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);
    manager
        .accept_message(&a, dummy_message())
        .await
        .expect("A handles a request after B was created");
    clock.advance(TEST_MIN_IDLE);

    manager
        .create_session()
        .await
        .expect("third session admitted by evicting an idle one");
    assert!(
        manager.is_live(&a),
        "A was active more recently than B, so A must survive"
    );
    assert!(
        !manager.is_live(&b),
        "B is the least recently active session, so B is the one evicted"
    );
}

/// A session with an open stream is never evicted, even if it is the
/// least-recently-active session by timestamp — busy overrides recency.
#[tokio::test]
async fn a_session_with_an_open_stream_is_never_evicted() {
    let (manager, clock) = bounded(2);
    let busy = open_initialized(&manager).await;
    let stream = manager
        .create_stream(&busy, dummy_message())
        .await
        .expect("open a stream on the oldest session");
    let idle = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    let (newcomer, _t) = manager
        .create_session()
        .await
        .expect("third session admitted by evicting the only idle candidate");
    assert!(
        manager.is_live(&busy),
        "the busy session must survive despite being the oldest"
    );
    assert!(
        !manager.is_live(&idle),
        "the idle session must be the one evicted, even though it is newer"
    );

    // Dropping the stream releases the guard, so the now-idle session
    // becomes evictable again once the floor has passed. The newcomer
    // finishes its handshake afterwards, so it is the more recently active
    // of the two and the formerly-busy session is the LRU candidate.
    drop(stream);
    manager
        .initialize_session(&newcomer, dummy_message())
        .await
        .expect("initialize the newcomer");
    clock.advance(TEST_MIN_IDLE);
    manager
        .create_session()
        .await
        .expect("fourth session admitted now that the stream closed");
    assert!(
        !manager.is_live(&busy),
        "once its stream closes, the formerly-busy session is evictable like any other"
    );
}

/// rmcp creates a session, spawns its worker, and only then initializes it.
/// A session in that gap is not idle: evicting it would fail its own
/// `initialize`. With A busy and B created but not initialized, the cap has
/// nothing to reclaim — even once B is older than the idle floor.
#[tokio::test]
async fn a_session_awaiting_initialize_is_not_evicted_as_idle() {
    let (manager, clock) = bounded(2);
    let a = open_initialized(&manager).await;
    let stream_a = manager
        .create_stream(&a, dummy_message())
        .await
        .expect("keep A busy");
    let (b, _tb) = manager.create_session().await.expect("create B");
    clock.advance(TEST_MIN_IDLE);

    let err = manager
        .create_session()
        .await
        .expect_err("C must be refused: A is busy and B has not finished initializing");
    assert!(err.is_too_many_sessions(), "{err}");
    assert!(
        manager.is_live(&b),
        "B must still be live for its initialize"
    );

    manager
        .initialize_session(&b, dummy_message())
        .await
        .expect("B initializes");
    clock.advance(TEST_MIN_IDLE);
    manager
        .create_session()
        .await
        .expect("once B's handshake is over and it has idled, it is evictable");
    assert!(!manager.is_live(&b));
    drop(stream_a);
}

/// A failed handshake must not leave the session protected forever: the
/// birth flag is cleared on entry, whatever `initialize` answers.
#[tokio::test]
async fn a_failed_initialize_still_leaves_the_session_evictable() {
    let inner = FakeSessionManager {
        initialize: FakeInitialize::Fails,
        ..FakeSessionManager::default()
    };
    let (manager, clock) = bounded_over(inner, 1);
    let (a, _ta) = manager.create_session().await.expect("create A");
    manager
        .initialize_session(&a, dummy_message())
        .await
        .expect_err("the fake refuses the handshake");
    clock.advance(TEST_MIN_IDLE);

    manager
        .create_session()
        .await
        .expect("A's failed handshake is over, so A is evictable");
    assert!(!manager.is_live(&a));
}

/// A handshake whose request is dropped at its first await must not leave
/// the session protected by its birth flag. The admission lock is held, as a
/// concurrent admission would hold it, while the handshake is polled once and
/// dropped: had `initialize_session` waited on that lock before clearing the
/// flag, the session would stay unevictable however long it then sat idle.
#[tokio::test]
async fn a_handshake_dropped_at_its_first_await_leaves_the_session_evictable() {
    let inner = FakeSessionManager {
        initialize: FakeInitialize::Hangs,
        ..FakeSessionManager::default()
    };
    let (manager, clock) = bounded_over(inner, 1);
    let (a, _ta) = manager.create_session().await.expect("create A");
    {
        let _concurrent_admission = manager.admission.lock().await;
        let handshake = manager.initialize_session(&a, dummy_message());
        assert!(
            handshake.now_or_never().is_none(),
            "the handshake is still in progress when its request is dropped"
        );
    }
    clock.advance(TEST_MIN_IDLE * 100);

    manager
        .create_session()
        .await
        .expect("A's abandoned handshake is over, so A is evictable");
    assert!(!manager.is_live(&a));
}

/// A session whose handshake never happened is abandoned once the inner
/// manager's `init_timeout` has passed (rmcp's restore path can leave such a
/// session with nothing left to close it): it becomes evictable then, one
/// nanosecond later than it would be protected, and not before.
#[tokio::test]
async fn a_session_never_initialized_is_evictable_after_the_init_timeout() {
    let (manager, clock) = bounded(1);
    let (a, _ta) = manager.create_session().await.expect("create A");

    clock.advance(TEST_INIT_TIMEOUT - Duration::from_nanos(1));
    let err = manager
        .create_session()
        .await
        .expect_err("A's handshake may still arrive, so A is not evictable");
    assert!(err.is_too_many_sessions(), "{err}");
    assert!(manager.is_live(&a));

    clock.advance(Duration::from_nanos(1));
    manager
        .create_session()
        .await
        .expect("A's handshake deadline has passed, so A is abandoned and evictable");
    assert!(!manager.is_live(&a));
}

/// An eviction's victim is marked closing in the step that picks it: a request
/// arriving while the close is still running is refused (and `has_session`
/// says the session is gone, so the transport answers 404) instead of being
/// admitted onto a session about to be closed.
#[tokio::test]
async fn a_session_picked_for_eviction_admits_no_new_request() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let inner = FakeSessionManager {
        close_gate: Some(Arc::clone(&gate)),
        ..FakeSessionManager::default()
    };
    let (manager, clock) = bounded_over(inner, 1);
    let a = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    let mut admission = Box::pin(manager.create_session());
    assert!(
        futures::poll!(admission.as_mut()).is_pending(),
        "the eviction is waiting on A's close"
    );
    assert!(
        !manager.has_session(&a).await.expect("has_session answers"),
        "a session being evicted must look gone to the transport"
    );
    let refused = manager.create_standalone_stream(&a).await.err();
    assert!(
        matches!(refused, Some(BoundedSessionManagerError::SessionClosing)),
        "no stream may be opened on a session being evicted: {refused:?}"
    );

    gate.add_permits(1);
    admission
        .await
        .expect("the newcomer is admitted once A is closed");
    assert!(!manager.is_live(&a));
}

/// A request reaches a session in two steps: rmcp's `has_session`, then the
/// call that serves it. An eviction landing between the two must not pick the
/// session and fail that call: the check stamps the session active, so it is
/// no longer past the idle floor and the newcomer is refused instead.
#[tokio::test]
async fn a_session_a_request_has_just_checked_is_not_picked_for_eviction() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let inner = FakeSessionManager {
        close_gate: Some(Arc::clone(&gate)),
        ..FakeSessionManager::default()
    };
    let (manager, clock) = bounded_over(inner, 1);
    let a = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    assert!(manager.has_session(&a).await.expect("has_session answers"));
    let admission = manager.create_session().now_or_never();
    assert!(
        matches!(&admission, Some(Err(err)) if err.is_too_many_sessions()),
        "A was just checked by a request, so the newcomer must be refused, not evict A: \
         {admission:?}"
    );
    let served = manager.create_stream(&a, dummy_message()).await.err();
    assert!(
        served.is_none(),
        "the request that checked A must be served: {served:?}"
    );
}

/// An eviction abandoned mid-close (its caller dropped) hands the victim back:
/// the closing mark is lifted and the session serves requests again.
#[tokio::test]
async fn an_eviction_dropped_mid_close_hands_the_session_back() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let inner = FakeSessionManager {
        close_gate: Some(Arc::clone(&gate)),
        ..FakeSessionManager::default()
    };
    let (manager, clock) = bounded_over(inner, 1);
    let a = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    assert!(
        manager.create_session().now_or_never().is_none(),
        "the eviction is waiting on A's close when it is dropped"
    );
    assert!(manager.is_live(&a));
    assert!(manager.has_session(&a).await.expect("has_session answers"));
    manager
        .accept_message(&a, dummy_message())
        .await
        .expect("A serves requests again once the eviction is abandoned");
}

/// Recency is stamped when activity ENDS: a stream opened long ago that
/// closed a moment ago leaves its session young, not ten floors old.
#[tokio::test]
async fn recency_is_stamped_when_activity_ends_not_when_it_starts() {
    let (manager, clock) = bounded(1);
    let a = open_initialized(&manager).await;
    let stream = manager
        .create_stream(&a, dummy_message())
        .await
        .expect("open a stream on A");
    clock.advance(TEST_MIN_IDLE * 10);
    drop(stream);

    let err = manager
        .create_session()
        .await
        .expect_err("A's stream ended just now, so A is not evictable");
    assert!(err.is_too_many_sessions(), "{err}");
    assert!(manager.is_live(&a));
}

/// Both sides of the eviction floor, one nanosecond apart: a session quiet
/// for less than `min_idle` is not evicted — the newcomer is refused exactly
/// as before #2289 — and the same session one tick later is.
#[tokio::test]
async fn only_a_session_idle_for_the_minimum_age_is_evicted() {
    let (manager, clock) = bounded(1);
    let a = open_initialized(&manager).await;

    clock.advance(TEST_MIN_IDLE - Duration::from_nanos(1));
    let err = manager
        .create_session()
        .await
        .expect_err("A has been idle for less than the floor, so nothing is evictable");
    assert!(err.is_too_many_sessions(), "{err}");
    assert!(manager.is_live(&a), "A must survive the refusal");

    clock.advance(Duration::from_nanos(1));
    manager
        .create_session()
        .await
        .expect("A has now been idle for exactly the floor, so it is evicted");
    assert!(!manager.is_live(&a));
}

/// Idle age is measured from the END of the last activity: a session idle
/// for ages that just handled a request is young again.
#[tokio::test]
async fn activity_restarts_the_idle_age() {
    let (manager, clock) = bounded(1);
    let a = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE * 10);
    manager
        .accept_message(&a, dummy_message())
        .await
        .expect("A handles a request");

    manager
        .create_session()
        .await
        .expect_err("A was active just now, so it is not evictable");
    assert!(manager.is_live(&a));
}

/// `LocalSessionManager` (like the trait default) answers `NotSupported` to
/// a restore: nothing is created, so nothing may be evicted for it.
#[tokio::test]
async fn a_restore_that_creates_nothing_evicts_nothing() {
    let (manager, clock) = bounded(1);
    let a = open_initialized(&manager).await;
    clock.advance(TEST_MIN_IDLE);

    let outcome = manager
        .restore_session("unknown-to-the-store".to_string().into())
        .await
        .expect("restore answers");
    assert!(matches!(outcome, RestoreOutcome::NotSupported));
    assert!(
        manager.is_live(&a),
        "an idle session must not be evicted to make room for a restore that created nothing"
    );
}

/// A genuine restore takes a slot like a creation does: at the cap it evicts
/// an evictable session, and when none is, it is refused and the restored
/// session closed again rather than admitted past the cap.
#[tokio::test]
async fn a_genuine_restore_at_the_cap_evicts_or_is_undone() {
    let inner = FakeSessionManager {
        restores: true,
        ..FakeSessionManager::default()
    };
    let (manager, clock) = bounded_over(inner, 1);
    let a = open_initialized(&manager).await;

    let young: SessionId = "restored-while-a-is-young".to_string().into();
    let err = manager
        .restore_session(young.clone())
        .await
        .expect_err("A is younger than the floor, so the restore cannot be admitted");
    assert!(err.is_too_many_sessions(), "{err}");
    assert!(!manager.is_live(&young));
    assert!(
        !manager
            .inner
            .has_session(&young)
            .await
            .expect("fake answers"),
        "the refused restore must be closed in the inner manager too"
    );

    clock.advance(TEST_MIN_IDLE);
    let old: SessionId = "restored-after-a-idled".to_string().into();
    let outcome = manager
        .restore_session(old.clone())
        .await
        .expect("A is evictable now");
    assert!(matches!(outcome, RestoreOutcome::Restored(_)));
    assert!(!manager.is_live(&a), "A made room for the restore");
    assert!(manager.is_live(&old));
}

#[tokio::test]
async fn closing_a_session_frees_a_slot_for_a_new_one() {
    let (manager, _clock) = bounded(1);
    let (id, _transport) = manager.create_session().await.expect("first session");
    let stream = manager
        .create_stream(&id, dummy_message())
        .await
        .expect("keep the only session busy so the cap genuinely refuses");
    manager
        .create_session()
        .await
        .expect_err("second session must be refused while the first is live and busy");

    drop(stream);
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
    // FakeSessionManager::create_session succeeds unless told to fail; make
    // it fail and confirm no reservation is consumed — i.e. the bound isn't
    // silently eaten by inner failures.
    let inner = FakeSessionManager {
        fails_create: true,
        ..FakeSessionManager::default()
    };
    let (manager, _clock) = bounded_over(inner, 1);
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
    let (manager, _clock) = bounded(2);
    let (a, _ta) = manager.create_session().await.expect("session A");
    let (_b, _tb) = manager.create_session().await.expect("session B");
    assert_eq!(manager.live_count(), 2);

    manager.close_session(&a).await.expect("first close");
    manager.close_session(&a).await.expect("second close");

    assert_eq!(
        manager.live_count(),
        1,
        "closing ONE session twice must still free exactly one slot"
    );
    manager
        .create_session()
        .await
        .expect("the freed slot must be reusable");
    assert_eq!(
        manager.live_count(),
        2,
        "only ONE slot was freed, so a second admission must never push past the cap"
    );
}

#[tokio::test]
async fn closing_an_unknown_session_frees_nothing() {
    let (manager, _clock) = bounded(1);
    let (a, _ta) = manager.create_session().await.expect("session A");
    let stream = manager
        .create_stream(&a, dummy_message())
        .await
        .expect("keep A busy so a stray close can't be mistaken for a freed slot");

    let stranger: SessionId = "never-created".to_string().into();
    manager
        .close_session(&stranger)
        .await
        .expect("closing an unknown id is a no-op, not an error");

    assert_eq!(manager.live_count(), 1);
    manager
        .create_session()
        .await
        .expect_err("an unknown id must not free the live, busy session's slot");

    drop(stream);
}

#[tokio::test]
async fn many_create_then_close_cycles_never_exhaust_the_bound() {
    // The guarantee that matters in production: a daemon cycling sessions
    // far more times than `max_sessions` must never lock itself out.
    let (manager, _clock) = bounded(2);
    for cycle in 0..64 {
        let (id, _t) = manager
            .create_session()
            .await
            .unwrap_or_else(|_| panic!("cycle {cycle} must still be able to open a session"));
        manager.close_session(&id).await.expect("close");
    }
    assert_eq!(manager.live_count(), 0);
}
