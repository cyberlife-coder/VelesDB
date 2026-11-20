//! Global per-IP rate limiting middleware backed by `tower-governor`.
//!
//! Provides a token-bucket rate limiter keyed by client IP address.
//! When the bucket is exhausted the server replies with `429 Too Many Requests`
//! and standard rate-limit headers (`x-ratelimit-limit`, `x-ratelimit-remaining`,
//! `retry-after`).

use std::sync::Arc;
use std::time::Duration;
use tower_governor::governor::{GovernorConfig, GovernorConfigBuilder};
use tower_governor::key_extractor::SmartIpKeyExtractor;

/// Re-export so callers can build a `GovernorLayer` from the config.
pub use tower_governor::GovernorLayer;

/// The middleware type used when `use_headers()` is enabled.
type HeaderMiddleware = ::governor::middleware::StateInformationMiddleware;

/// Concrete governor config type with per-IP keying and rate-limit headers.
pub type RateLimitConfig = GovernorConfig<SmartIpKeyExtractor, HeaderMiddleware>;

/// The interval after which one request is given back to a client that may
/// send `requests_per_second`: a `requests_per_second`-th of a second.
///
/// `None` for a rate of zero, which has no interval. Above a billion requests
/// per second the division rounds to nothing, so the interval is held at one
/// nanosecond — the builder refuses a zero interval.
fn refill_interval(requests_per_second: u32) -> Option<Duration> {
    Duration::from_secs(1)
        .checked_div(requests_per_second)
        .map(|interval| interval.max(Duration::from_nanos(1)))
}

/// Build a [`GovernorConfig`] that enforces `burst` requests/second per IP.
///
/// A client may send `burst` requests at once and is given one back every
/// `1 / burst` seconds, so a client that keeps to `burst` per second is never
/// refused and an emptied bucket is full again a second later.
///
/// Uses [`SmartIpKeyExtractor`] which inspects `x-forwarded-for`,
/// `x-real-ip`, `forwarded` headers before falling back to the peer IP,
/// making it safe behind reverse proxies.
///
/// A background thread periodically prunes stale entries from the
/// governor limiter map (every 60 s).
///
/// # Errors
///
/// Returns an error if `burst` is zero or the governor configuration cannot
/// be built.
pub fn build_rate_limit_config(burst: u32) -> anyhow::Result<Arc<RateLimitConfig>> {
    let interval = refill_interval(burst)
        .ok_or_else(|| anyhow::anyhow!("a rate limit of 0 requests per second has no bucket"))?;
    let mut builder = GovernorConfigBuilder::default();
    // `period` is the time one request takes to come back. The builder's
    // `per_second(n)` is that same interval expressed in seconds, not a rate:
    // `per_second(burst)` gave one request back every `burst` seconds.
    builder.period(interval);
    builder.burst_size(burst);
    let mut builder = builder.key_extractor(SmartIpKeyExtractor);
    let mut builder = builder.use_headers();

    let config = Arc::new(
        builder
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build rate limiter configuration"))?,
    );

    spawn_limiter_cleanup(&config);

    Ok(config)
}

/// Spawns a background thread that prunes stale rate-limiter entries every 60 s.
fn spawn_limiter_cleanup(config: &Arc<RateLimitConfig>) {
    let limiter = config.limiter().clone();
    let interval = Duration::from_secs(60);
    std::thread::spawn(move || loop {
        std::thread::sleep(interval);
        tracing::debug!("rate limiter cleanup: {} tracked IPs", limiter.len());
        limiter.retain_recent();
    });
}

#[cfg(test)]
#[path = "rate_limit_tests.rs"]
mod tests;
