//! Integration tests for update_check module

use super::*;

#[test]
fn test_compute_instance_hash_returns_valid_hash() {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    let hash = compute_instance_hash(temp_dir.path());

    // SHA256 hex = 64 chars
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_update_check_config_defaults() {
    let config = UpdateCheckConfig::default();

    assert!(config.enabled);
    assert_eq!(config.endpoint, "https://velesdb.com/api/check");
    assert_eq!(config.timeout_ms, 2000);
}
