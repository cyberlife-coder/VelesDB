//! Tests for `guardrails` module - Query limits, rate limiting, circuit breaker.

use super::guardrails::*;

#[test]
fn test_query_limits_default() {
    let limits = QueryLimits::default();
    assert_eq!(limits.max_depth, DEFAULT_MAX_DEPTH);
    assert_eq!(limits.max_cardinality, DEFAULT_MAX_CARDINALITY);
}

#[test]
fn test_query_context_depth_check() {
    let ctx = QueryContext::new(QueryLimits::default().with_max_depth(5));
    assert!(ctx.check_depth(3).is_ok());
    assert!(ctx.check_depth(5).is_ok());
    assert!(ctx.check_depth(6).is_err());
}

#[test]
fn test_query_context_cardinality_check() {
    let ctx = QueryContext::new(QueryLimits::default().with_max_cardinality(100));
    assert!(ctx.check_cardinality(50).is_ok());
    assert!(ctx.check_cardinality(40).is_ok());
    assert!(ctx.check_cardinality(20).is_err());
}

#[test]
fn test_query_context_memory_check() {
    let ctx = QueryContext::new(QueryLimits::default().with_memory_limit(1000));
    assert!(ctx.check_memory(500).is_ok());
    assert!(ctx.check_memory(400).is_ok());
    assert!(ctx.check_memory(200).is_err());
}

#[test]
fn test_rate_limiter() {
    let limiter = RateLimiter::new(2);
    assert!(limiter.check("client1").is_ok());
    assert!(limiter.check("client1").is_ok());
    assert!(limiter.check("client1").is_err());
    assert!(limiter.check("client2").is_ok());
}

#[test]
fn test_circuit_breaker() {
    let cb = CircuitBreaker::new(2, 1);
    assert!(cb.check().is_ok());
    assert_eq!(cb.state(), CircuitState::Closed);

    cb.record_failure();
    assert_eq!(cb.state(), CircuitState::Closed);

    cb.record_failure();
    assert_eq!(cb.state(), CircuitState::Open);
    assert!(cb.check().is_err());
}

#[test]
fn test_circuit_breaker_recovery() {
    let cb = CircuitBreaker::new(1, 0);
    cb.record_failure();
    assert_eq!(cb.state(), CircuitState::Open);

    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(cb.check().is_ok());
    assert_eq!(cb.state(), CircuitState::HalfOpen);

    cb.record_success();
    assert_eq!(cb.state(), CircuitState::Closed);
}

#[test]
fn test_guard_rails_pre_check() {
    let gr = GuardRails::new();
    assert!(gr.pre_check("client1").is_ok());
}
