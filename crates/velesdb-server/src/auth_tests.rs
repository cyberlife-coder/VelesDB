use super::*;

#[test]
fn test_auth_state_disabled_when_empty() {
    let state = AuthState::new(vec![]);
    assert!(!state.auth_enabled());
}

#[test]
fn test_auth_state_enabled_with_keys() {
    let state = AuthState::new(vec!["key1".to_string()]);
    assert!(state.auth_enabled());
}

#[test]
fn test_is_public_path_health() {
    assert!(is_public_path("/health"));
}

#[test]
fn test_is_public_path_ready() {
    assert!(is_public_path("/ready"));
}

#[test]
fn test_is_public_path_metrics_is_protected() {
    // F-02: /metrics must not bypass authentication — it leaks
    // operational details about the running database.
    assert!(!is_public_path("/metrics"));
    assert!(!is_public_path("/v1/metrics"));
}

#[test]
fn test_is_public_path_versioned_health() {
    assert!(is_public_path("/v1/health"));
}

#[test]
fn test_is_public_path_versioned_ready() {
    assert!(is_public_path("/v1/ready"));
}

#[test]
fn test_is_public_path_other() {
    assert!(!is_public_path("/collections"));
    assert!(!is_public_path("/query"));
    assert!(!is_public_path("/health/extra"));
    assert!(!is_public_path("/v1/collections"));
}

#[test]
fn test_extract_bearer_token_valid() {
    assert_eq!(extract_bearer_token("Bearer my-key"), Some("my-key"));
    assert_eq!(extract_bearer_token("bearer my-key"), Some("my-key"));
    assert_eq!(extract_bearer_token("BEARER my-key"), Some("my-key"));
    assert_eq!(extract_bearer_token("  Bearer  my-key  "), Some("my-key"));
}

#[test]
fn test_extract_bearer_token_invalid() {
    assert_eq!(extract_bearer_token("Basic abc123"), None);
    assert_eq!(extract_bearer_token("my-key"), None);
    assert_eq!(extract_bearer_token("Bearer"), None);
    assert_eq!(extract_bearer_token(""), None);
}

#[test]
fn test_extract_bearer_token_whitespace_only() {
    assert_eq!(extract_bearer_token("Bearer   "), None);
}

// ========================================================================
// Constant-time comparison tests
// ========================================================================

#[test]
fn test_constant_time_eq_identical() {
    assert!(constant_time_eq(b"secret-key-42", b"secret-key-42"));
}

#[test]
fn test_constant_time_eq_different_content() {
    assert!(!constant_time_eq(b"secret-key-42", b"secret-key-43"));
}

#[test]
fn test_constant_time_eq_different_length() {
    assert!(!constant_time_eq(b"short", b"longer-key"));
}

#[test]
fn test_constant_time_eq_empty() {
    assert!(constant_time_eq(b"", b""));
}

#[test]
fn test_any_key_matches_found() {
    let keys = vec!["key-a".to_string(), "key-b".to_string()];
    assert!(any_key_matches(&keys, "key-b"));
}

#[test]
fn test_any_key_matches_not_found() {
    let keys = vec!["key-a".to_string(), "key-b".to_string()];
    assert!(!any_key_matches(&keys, "key-c"));
}

#[test]
fn test_any_key_matches_empty_keys() {
    let keys: Vec<String> = vec![];
    assert!(!any_key_matches(&keys, "anything"));
}
