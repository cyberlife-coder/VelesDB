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

#[test]
fn test_guard_rails_update_limits() {
    let gr = GuardRails::new();
    assert_eq!(gr.limits().timeout_ms, 30_000);

    let new_limits = QueryLimits::default().with_timeout_ms(5_000);
    gr.update_limits(&new_limits);

    assert_eq!(gr.limits().timeout_ms, 5_000);
}

#[test]
fn test_guard_rails_update_limits_propagates_to_context() {
    let gr = GuardRails::new();

    let new_limits = QueryLimits::default()
        .with_max_depth(3)
        .with_timeout_ms(1_000);
    gr.update_limits(&new_limits);

    // Contexts created after update should use the new limits.
    let ctx = gr.create_context();
    assert!(ctx.check_depth(3).is_ok());
    assert!(ctx.check_depth(4).is_err());
}

#[test]
fn test_guard_rails_update_preserves_circuit_breaker_state() {
    let gr = GuardRails::with_limits(QueryLimits::default().with_timeout_ms(30_000));
    // Trip the circuit breaker.
    for _ in 0..5 {
        gr.circuit_breaker.record_failure();
    }
    assert_eq!(gr.circuit_breaker.state(), CircuitState::Open);

    // Update limits should not reset circuit breaker.
    gr.update_limits(&QueryLimits::default().with_timeout_ms(1_000));
    assert_eq!(gr.circuit_breaker.state(), CircuitState::Open);
}
