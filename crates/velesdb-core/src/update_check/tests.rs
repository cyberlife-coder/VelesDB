//! Integration tests for update_check module

use super::*;

#[test]
fn test_update_check_config_defaults() {
    let config = UpdateCheckConfig::default();

    assert!(config.enabled);
    assert_eq!(config.endpoint, "https://velesdb.com/api/check");
    assert_eq!(config.timeout_ms, 2000);
}
