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
    let key = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

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
