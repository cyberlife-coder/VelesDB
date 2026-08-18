use super::*;
use velesdb_core::guardrails::QueryLimits;

#[test]
fn test_limits_to_response_roundtrip() {
    let limits = QueryLimits::default();
    let response = limits_to_response(&limits);
    assert_eq!(response.max_depth, limits.max_depth);
    assert_eq!(response.max_cardinality, limits.max_cardinality);
    assert_eq!(response.memory_limit_bytes, limits.memory_limit_bytes);
    assert_eq!(response.timeout_ms, limits.timeout_ms);
    assert_eq!(response.rate_limit_qps, limits.rate_limit_qps);
    assert_eq!(
        response.circuit_failure_threshold,
        limits.circuit_failure_threshold
    );
    assert_eq!(
        response.circuit_recovery_seconds,
        limits.circuit_recovery_seconds
    );
}

#[test]
fn test_apply_guardrails_partial_update() {
    let mut limits = QueryLimits::default();
    let original_timeout = limits.timeout_ms;

    let req = GuardRailsConfigRequest {
        max_depth: Some(20),
        max_cardinality: None,
        memory_limit_bytes: None,
        timeout_ms: None,
        rate_limit_qps: Some(500),
        circuit_failure_threshold: None,
        circuit_recovery_seconds: None,
    };

    apply_guardrails_update(&mut limits, &req);

    assert_eq!(limits.max_depth, 20);
    assert_eq!(limits.rate_limit_qps, 500);
    // Unchanged fields remain at defaults
    assert_eq!(limits.timeout_ms, original_timeout);
}

#[test]
fn test_apply_guardrails_full_update() {
    let mut limits = QueryLimits::default();

    let req = GuardRailsConfigRequest {
        max_depth: Some(5),
        max_cardinality: Some(50_000),
        memory_limit_bytes: Some(1024 * 1024),
        timeout_ms: Some(10_000),
        rate_limit_qps: Some(200),
        circuit_failure_threshold: Some(3),
        circuit_recovery_seconds: Some(60),
    };

    apply_guardrails_update(&mut limits, &req);

    assert_eq!(limits.max_depth, 5);
    assert_eq!(limits.max_cardinality, 50_000);
    assert_eq!(limits.memory_limit_bytes, 1024 * 1024);
    assert_eq!(limits.timeout_ms, 10_000);
    assert_eq!(limits.rate_limit_qps, 200);
    assert_eq!(limits.circuit_failure_threshold, 3);
    assert_eq!(limits.circuit_recovery_seconds, 60);
}

#[test]
fn test_guardrails_response_serialization() {
    let response = GuardRailsConfigResponse {
        max_depth: 10,
        max_cardinality: 100_000,
        memory_limit_bytes: 104_857_600,
        timeout_ms: 30_000,
        rate_limit_qps: 100,
        circuit_failure_threshold: 5,
        circuit_recovery_seconds: 30,
    };
    let json = serde_json::to_string(&response).expect("serialize");
    assert!(json.contains("\"max_depth\":10"));
    assert!(json.contains("\"rate_limit_qps\":100"));
}
