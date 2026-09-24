use super::*;

#[test]
fn test_session_defaults() {
    let session = SessionSettings::new();
    // No mode of its own: the configured default applies, and `\show` says so.
    assert_eq!(session.search_quality(), None);
    assert_eq!(session.mode_str(), None);
    assert_eq!(
        session.get("mode", SearchQuality::Fast).as_deref(),
        Some("fast (configured default)")
    );
    assert_eq!(
        session.get("ef_search", SearchQuality::Fast).as_deref(),
        Some("auto (96)")
    );
    assert_eq!(session.timeout_ms(), 30000);
    assert!(session.rerank());
    assert_eq!(session.max_results(), 100);
    assert!(session.active_collection().is_none());
}

#[test]
fn test_set_mode() {
    let mut session = SessionSettings::new();
    session.set("mode", "fast").unwrap();
    assert_eq!(session.search_quality(), Some(SearchQuality::Fast));
    // The session's mode, not the configured one, sets the shown ef_search.
    assert_eq!(
        session.get("ef_search", SearchQuality::Accurate).as_deref(),
        Some("auto (96)")
    );
}

#[test]
fn test_set_ef_search() {
    let mut session = SessionSettings::new();
    session.set("ef_search", "512").unwrap();
    assert_eq!(session.search_quality(), Some(SearchQuality::Custom(512)));
    assert_eq!(
        session.get("ef_search", SearchQuality::Balanced).as_deref(),
        Some("512")
    );
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
    assert_eq!(session.search_quality(), None);
}

#[test]
fn test_reset_all() {
    let mut session = SessionSettings::new();
    session.set("mode", "fast").unwrap();
    session.set("ef_search", "512").unwrap();
    session.reset(None);
    assert_eq!(session.search_quality(), None);
}

#[test]
fn test_all_settings() {
    let session = SessionSettings::new();
    let settings = session.all_settings(SearchQuality::Balanced);
    assert!(settings.iter().any(|(k, _)| k == "mode"));
    assert!(settings.iter().any(|(k, _)| k == "ef_search"));
}

#[test]
fn test_get_setting() {
    let session = SessionSettings::new();
    assert_eq!(
        session.get("mode", SearchQuality::Balanced).as_deref(),
        Some("balanced (configured default)")
    );
    assert!(session.get("unknown", SearchQuality::Balanced).is_none());
}

#[test]
fn test_custom_setting() {
    let mut session = SessionSettings::new();
    session.set("custom_key", "custom_value").unwrap();
    assert_eq!(
        session.get("custom_key", SearchQuality::Balanced),
        Some("custom_value".to_string())
    );
}

#[test]
fn test_set_mode_autotune() {
    let mut session = SessionSettings::new();
    session.set("mode", "autotune").unwrap();
    assert_eq!(session.search_quality(), Some(SearchQuality::AutoTune));
}

#[test]
fn test_set_mode_custom() {
    let mut session = SessionSettings::new();
    session.set("mode", "custom:256").unwrap();
    assert_eq!(session.search_quality(), Some(SearchQuality::Custom(256)));
    assert_eq!(
        session.get("ef_search", SearchQuality::Balanced).as_deref(),
        Some("auto (256)")
    );
}

#[test]
fn test_set_mode_adaptive() {
    let mut session = SessionSettings::new();
    session.set("mode", "adaptive:32:512").unwrap();
    assert_eq!(
        session.search_quality(),
        Some(SearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512
        })
    );
}

#[test]
fn test_set_mode_invalid() {
    let mut session = SessionSettings::new();
    assert!(session.set("mode", "nonexistent").is_err());
    assert!(session.set("mode", "custom:abc").is_err());
    assert!(session.set("mode", "adaptive:32").is_err());
    // min_ef above max_ef: every query would refuse it, so `\set` does (#2267).
    assert!(session.set("mode", "adaptive:512:32").is_err());
}

#[test]
fn test_set_mode_custom_out_of_range_is_rejected() {
    // An unbounded ef would reach the same uncapped HNSW traversal #2274
    // closes for the dedicated `ef_search` option — `\set mode custom:<ef>`
    // must refuse it the same way (#2275).
    let mut session = SessionSettings::new();
    assert!(session.set("mode", "custom:99999999").is_err());
    assert!(session.set("mode", "adaptive:32:99999999").is_err());
}

#[test]
fn test_search_quality_is_the_ef_search_when_set_else_the_mode_else_none() {
    let mut session = SessionSettings::new();
    assert_eq!(session.search_quality(), None);
    session.set("ef_search", "64").unwrap();
    assert_eq!(session.search_quality(), Some(SearchQuality::Custom(64)));
    // `\set mode` resets the ef_search override.
    session.set("mode", "fast").unwrap();
    assert_eq!(session.search_quality(), Some(SearchQuality::Fast));
}
