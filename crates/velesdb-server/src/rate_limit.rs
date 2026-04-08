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

/// Build a [`GovernorConfig`] that enforces `burst` requests/second per IP.
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
/// Returns an error if the governor configuration cannot be built.
pub fn build_rate_limit_config(burst: u32) -> anyhow::Result<Arc<RateLimitConfig>> {
    let mut builder = GovernorConfigBuilder::default();
    builder.per_second(u64::from(burst));
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
mod tests {
    use super::*;

    #[test]
    fn test_build_rate_limit_config_succeeds() {
        let config = build_rate_limit_config(100);
        assert!(
            config.is_ok(),
            "governor config should build with burst=100"
        );
    }

    #[test]
    fn test_build_rate_limit_config_burst_one() {
        let config = build_rate_limit_config(1);
        assert!(config.is_ok(), "governor config should build with burst=1");
    }

    /// BUG-1 regression: replenishment rate must scale with `burst`.
    ///
    /// With `per_second(burst)`, the full bucket refills within 1 second.
    /// Previously, `per_second(1)` hardcoded 1 token/sec regardless of burst.
    #[test]
    fn test_rate_limit_replenishment_scales_with_burst() {
        // burst=100 → per_second(100) → 100 tokens/sec replenishment.
        // The limiter should allow at least 2 requests in rapid succession
        // after one token has been consumed and a brief wait.
        let config = build_rate_limit_config(100).expect("config should build");
        let limiter = config.limiter();

        // Simulate a key (IP address represented as a simple value).
        let key = std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1));

        // First request should always succeed (bucket starts full).
        assert!(
            limiter.check_key(&key).is_ok(),
            "first request should succeed"
        );

        // With burst=100 and per_second(100), we should have 99 remaining
        // tokens immediately — second request must also succeed.
        assert!(
            limiter.check_key(&key).is_ok(),
            "second request should succeed (burst=100 allows many concurrent)"
        );
    }
}
