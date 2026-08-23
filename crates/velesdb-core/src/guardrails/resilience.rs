//! Rate limiting and circuit breaker patterns (EPIC-048 US-005, US-006).
//!
//! Provides production-grade resilience primitives:
//! - **Token-bucket rate limiter** for per-client query throttling
//! - **Circuit breaker** for automatic failure protection

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::limits::GuardRailViolation;

// ─────────────────────────────────────────────────────────────────────────────
// Rate Limiter
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum number of distinct `client_id` buckets retained at once.
///
/// The per-client `TokenBucket` map is otherwise unbounded: an attacker
/// rotating `client_id` on every request would grow it without limit and
/// exhaust memory. When inserting a brand-new client would exceed this cap we
/// evict one existing bucket (see [`RateLimiter::evict_one`]) to make room.
/// Active clients within the cap keep accurate rate-limit state, and re-using
/// an already-tracked id always hits its existing bucket (eviction only ever
/// targets *other* buckets), so a client cannot reset its own throttle state.
pub(super) const MAX_TRACKED_CLIENTS: usize = 100_000;

/// Number of buckets examined per eviction.
///
/// Eviction inspects at most this many buckets (a bounded sample of the map,
/// effectively randomized by the `HashMap`'s `SipHash` seed) rather than
/// scanning all [`MAX_TRACKED_CLIENTS`] entries. This keeps eviction `O(k)`
/// on the hot path under the exclusive write lock, closing the low-QPS `DoS`
/// where every bucket is non-idle and a full scan would otherwise run on
/// every new-client request.
const EVICTION_SAMPLE_SIZE: usize = 16;

/// Rate limiter for query throttling (EPIC-048 US-005).
#[derive(Debug)]
pub struct RateLimiter {
    /// Tokens per second limit.
    limit_qps: std::sync::atomic::AtomicU32,
    /// Last check time per client.
    clients: parking_lot::RwLock<HashMap<String, TokenBucket>>,
}

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    last_update: Instant,
}

impl RateLimiter {
    /// Creates a new rate limiter with the given QPS limit.
    #[must_use]
    pub fn new(limit_qps: u32) -> Self {
        Self {
            limit_qps: std::sync::atomic::AtomicU32::new(limit_qps),
            clients: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Checks if a request from the given client is allowed.
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::RateLimitExceeded`] when the client has
    /// no available tokens in the current refill window.
    pub fn check(&self, client_id: &str) -> Result<(), GuardRailViolation> {
        let mut clients = self.clients.write();
        let now = Instant::now();
        let limit_qps = self.limit_qps.load(std::sync::atomic::Ordering::Relaxed);
        let limit = f64::from(limit_qps);

        // Bound the map before inserting a brand-new client so that a rotating
        // `client_id` attacker cannot grow it without limit (OOM guard).
        if !clients.contains_key(client_id) && clients.len() >= MAX_TRACKED_CLIENTS {
            Self::evict_one(&mut clients, limit);
        }

        let bucket = clients.entry(client_id.to_string()).or_insert(TokenBucket {
            tokens: limit,
            last_update: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * limit).min(limit);
        bucket.last_update = now;

        // Try to consume a token
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(GuardRailViolation::RateLimitExceeded { limit_qps })
        }
    }

    /// Evicts a single bucket to make room for a new client.
    ///
    /// Examines only a bounded sample of at most [`EVICTION_SAMPLE_SIZE`]
    /// buckets — *not* the whole map — so the work is `O(k)` regardless of how
    /// many clients are tracked. Within the sample it prefers an idle bucket
    /// (one that has fully refilled to `limit` given the elapsed time —
    /// dropping it is lossless since it is recreated full on demand); if none
    /// in the sample are idle it evicts the oldest-touched bucket in the
    /// sample. The sample is effectively randomized by the `HashMap`'s
    /// `SipHash` seed, so under sustained pressure every bucket is eventually
    /// a candidate and the map stays bounded even when all clients are active.
    ///
    /// This is safe against `client_id` rotation: eviction only runs when
    /// inserting a *new* id, and only ever removes some *other* bucket, so a
    /// client re-using a throttled id still hits its existing (throttled)
    /// bucket and cannot reset its own limit.
    fn evict_one(clients: &mut HashMap<String, TokenBucket>, limit: f64) {
        let now = Instant::now();
        let mut idle: Option<String> = None;
        let mut oldest: Option<(String, Instant)> = None;
        for (id, bucket) in clients.iter().take(EVICTION_SAMPLE_SIZE) {
            let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
            if (bucket.tokens + elapsed * limit).min(limit) >= limit {
                idle = Some(id.clone());
                break;
            }
            if oldest
                .as_ref()
                .is_none_or(|(_, ts)| bucket.last_update < *ts)
            {
                oldest = Some((id.clone(), bucket.last_update));
            }
        }
        if let Some(key) = idle.or_else(|| oldest.map(|(id, _)| id)) {
            clients.remove(&key);
        }
    }

    /// Returns the number of currently tracked client buckets.
    ///
    /// Test-only accessor used to assert the OOM guard keeps the map bounded.
    #[cfg(test)]
    pub(crate) fn tracked_clients(&self) -> usize {
        self.clients.read().len()
    }

    /// Exhausts all tokens for a specific client, ensuring the next
    /// `check()` for that client will fail.
    ///
    /// Only affects the targeted client's bucket — other clients and the
    /// global `limit_qps` are untouched.
    ///
    /// Useful for testing rate-limit rejection paths.
    pub fn exhaust(&self, client_id: &str) {
        let mut clients = self.clients.write();
        let now = Instant::now();
        let bucket = clients.entry(client_id.to_string()).or_insert(TokenBucket {
            tokens: 0.0,
            last_update: now,
        });
        // Set tokens to a large negative value so that even after time-based
        // refill the bucket stays below 1.0 for a reasonable test window.
        // With limit_qps = 100_000, it takes 10 seconds to refill 1M tokens.
        bucket.tokens = -1_000_000.0;
        bucket.last_update = now;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Circuit Breaker
// ─────────────────────────────────────────────────────────────────────────────

/// Circuit breaker state (EPIC-048 US-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CircuitState {
    /// Circuit is closed, requests are allowed.
    Closed,
    /// Circuit is open, requests are rejected.
    Open,
    /// Circuit is half-open, testing if service is healthy.
    HalfOpen,
}

/// Transition state and the instant the breaker opened, under one lock.
///
/// These two values are a single state machine: `opened_at` is only
/// meaningful while `state` is [`CircuitState::Open`], and every transition
/// reads or writes both. Holding them in separate locks made the acquisition
/// order differ per method — `check` took `opened_at` then `state`, while
/// `record_failure` took `state` then `opened_at` — an ABBA deadlock between
/// two paths the query pipeline calls on every request. Fusing them removes
/// the ordering question instead of documenting it.
#[derive(Debug, Clone, Copy)]
struct CircuitCore {
    /// Current state.
    state: CircuitState,
    /// Time when the circuit opened; `Some` exactly while `state` is `Open`.
    opened_at: Option<Instant>,
}

/// Circuit breaker for automatic failure protection (EPIC-048 US-006).
#[derive(Debug)]
pub struct CircuitBreaker {
    /// State machine (transition state + open instant) under a single lock.
    core: parking_lot::RwLock<CircuitCore>,
    /// Consecutive failure count.
    failure_count: AtomicU64,
    /// Failure threshold before opening.
    failure_threshold: u32,
    /// Recovery time in seconds.
    recovery_seconds: u64,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with the given configuration.
    #[must_use]
    pub fn new(failure_threshold: u32, recovery_seconds: u64) -> Self {
        Self {
            core: parking_lot::RwLock::new(CircuitCore {
                state: CircuitState::Closed,
                opened_at: None,
            }),
            failure_count: AtomicU64::new(0),
            failure_threshold,
            recovery_seconds,
        }
    }

    /// Checks if a request is allowed.
    ///
    /// # Errors
    ///
    /// Returns [`GuardRailViolation::CircuitOpen`] when the breaker is open and
    /// recovery time has not elapsed.
    pub fn check(&self) -> Result<(), GuardRailViolation> {
        // One read, copied out: the guard dies at the end of this statement,
        // so no lock is held while the transition below takes the write lock.
        let core = *self.core.read();
        if core.state != CircuitState::Open {
            return Ok(());
        }
        // `opened_at` is `Some` for every `Open` state the transitions produce;
        // a `None` here would mean a corrupt state machine, so allow the request
        // rather than reject traffic on an invariant we cannot repair.
        let Some(opened_at) = core.opened_at else {
            return Ok(());
        };

        let elapsed = opened_at.elapsed().as_secs();
        if elapsed < self.recovery_seconds {
            return Err(GuardRailViolation::CircuitOpen {
                recovery_in_seconds: self.recovery_seconds.saturating_sub(elapsed),
            });
        }

        // Transition to half-open. The read guard was released above, so
        // re-check the state: another thread may have moved the breaker in
        // between (this is a probe transition, not a lost update).
        let mut core = self.core.write();
        if core.state == CircuitState::Open {
            core.state = CircuitState::HalfOpen;
        }
        Ok(())
    }

    /// Records a successful request.
    pub fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        let mut core = self.core.write();
        if core.state == CircuitState::HalfOpen {
            core.state = CircuitState::Closed;
            // Keeps `opened_at.is_some() == (state == Open)` true. Unobservable
            // on its own (it is only read while `Open`), but the invariant is
            // now the type's, so it is upheld at every transition.
            core.opened_at = None;
        }
    }

    /// Records a failed request.
    pub fn record_failure(&self) {
        // Acquire the state write-lock BEFORE incrementing the counter to close the TOCTOU
        // window. Without this, a thread could compute count >= threshold, be preempted, and
        // then resume after the circuit has already gone through Open → HalfOpen — incorrectly
        // resetting opened_at and extending the recovery window.
        let mut core = self.core.write();
        let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= u64::from(self.failure_threshold)
            && (core.state == CircuitState::Closed || core.state == CircuitState::HalfOpen)
        {
            core.state = CircuitState::Open;
            core.opened_at = Some(Instant::now());
        }
    }

    /// Returns the current state.
    #[must_use]
    pub fn state(&self) -> CircuitState {
        self.core.read().state
    }
}
