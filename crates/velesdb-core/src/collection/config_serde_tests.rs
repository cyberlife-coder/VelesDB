use super::duration_secs;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Wrapper {
    #[serde(with = "duration_secs")]
    cooldown: Duration,
}

#[test]
fn config_serde_duration_roundtrip() {
    let original = Wrapper {
        cooldown: Duration::from_secs(3600),
    };

    let json = serde_json::to_string(&original).expect("serialize should succeed");
    // The on-disk form is a bare integer count of seconds, not a struct.
    assert_eq!(json, r#"{"cooldown":3600}"#);

    let restored: Wrapper = serde_json::from_str(&json).expect("deserialize should succeed");
    assert_eq!(restored, original);
    assert_eq!(restored.cooldown, Duration::from_secs(3600));
}

#[test]
fn config_serde_duration_drops_subsecond() {
    // Sub-second precision is intentionally dropped to whole seconds.
    let original = Wrapper {
        cooldown: Duration::from_millis(1500),
    };
    let json = serde_json::to_string(&original).expect("serialize should succeed");
    assert_eq!(json, r#"{"cooldown":1}"#);
}
