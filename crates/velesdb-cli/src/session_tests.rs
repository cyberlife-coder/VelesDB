use super::*;

#[test]
fn test_session_defaults() {
    let session = SessionSettings::new();
    assert_eq!(session.mode(), SearchQuality::Balanced);
    assert_eq!(session.effective_ef_search(), 160);
    assert_eq!(session.timeout_ms(), 30000);
    assert!(session.rerank());
    assert_eq!(session.max_results(), 100);
    assert!(session.active_collection().is_none());
}

#[test]
fn test_set_mode() {
    let mut session = SessionSettings::new();
    session.set("mode", "fast").unwrap();
    assert_eq!(session.mode(), SearchQuality::Fast);
    assert_eq!(session.effective_ef_search(), 96);
}

#[test]
fn test_set_ef_search() {
    let mut session = SessionSettings::new();
    session.set("ef_search", "512").unwrap();
    assert_eq!(session.effective_ef_search(), 512);
}

#[test]
fn test_set_ef_search_invalid_range() {
    let mut session = SessionSettings::new();
    assert!(session.set("ef_search", "10").is_err());
    assert!(session.set("ef_search", "5000").is_err());
}

#[test]
fn test_set_timeout() {
    let mut session = SessionSettings::new();
    session.set("timeout_ms", "5000").unwrap();
    assert_eq!(session.timeout_ms(), 5000);
}

#[test]
fn test_set_rerank() {
    let mut session = SessionSettings::new();
    session.set("rerank", "off").unwrap();
    assert!(!session.rerank());
    session.set("rerank", "true").unwrap();
    assert!(session.rerank());
}

#[test]
fn test_use_collection() {
    let mut session = SessionSettings::new();
    session.use_collection(Some("documents".to_string()));
    assert_eq!(session.active_collection(), Some("documents"));
}

#[test]
fn test_reset_single() {
    let mut session = SessionSettings::new();
    session.set("mode", "fast").unwrap();
    session.reset(Some("mode"));
    assert_eq!(session.mode(), SearchQuality::Balanced);
}

#[test]
fn test_reset_all() {
    let mut session = SessionSettings::new();
    session.set("mode", "fast").unwrap();
    session.set("ef_search", "512").unwrap();
    session.reset(None);
    assert_eq!(session.mode(), SearchQuality::Balanced);
    assert!(session.ef_search.is_none());
}

#[test]
fn test_all_settings() {
    let session = SessionSettings::new();
    let settings = session.all_settings();
    assert!(settings.iter().any(|(k, _)| k == "mode"));
    assert!(settings.iter().any(|(k, _)| k == "ef_search"));
}

#[test]
fn test_get_setting() {
    let session = SessionSettings::new();
    assert_eq!(session.get("mode"), Some("balanced".to_string()));
    assert!(session.get("unknown").is_none());
}

#[test]
fn test_custom_setting() {
    let mut session = SessionSettings::new();
    session.set("custom_key", "custom_value").unwrap();
    assert_eq!(session.get("custom_key"), Some("custom_value".to_string()));
}

#[test]
fn test_set_mode_autotune() {
    let mut session = SessionSettings::new();
    session.set("mode", "autotune").unwrap();
    assert_eq!(session.mode(), SearchQuality::AutoTune);
}

#[test]
fn test_set_mode_custom() {
    let mut session = SessionSettings::new();
    session.set("mode", "custom:256").unwrap();
    assert_eq!(session.mode(), SearchQuality::Custom(256));
    assert_eq!(session.effective_ef_search(), 256);
}

#[test]
fn test_set_mode_adaptive() {
    let mut session = SessionSettings::new();
    session.set("mode", "adaptive:32:512").unwrap();
    assert_eq!(
        session.mode(),
        SearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512
        }
    );
}

#[test]
fn test_set_mode_invalid() {
    let mut session = SessionSettings::new();
    assert!(session.set("mode", "nonexistent").is_err());
    assert!(session.set("mode", "custom:abc").is_err());
    assert!(session.set("mode", "adaptive:32").is_err());
}
