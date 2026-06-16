//! Serde helpers for persisting collection-config fields with non-default
//! on-disk representations.
//!
//! The only helper here serializes [`std::time::Duration`] as a whole number
//! of seconds (`u64`). `serde`'s default `Duration` representation is a
//! `{ secs, nanos }` struct, which is verbose and couples the on-disk format
//! to an implementation detail. Persisting seconds keeps `config.json` human
//! readable and stable across `Duration` internals.
//!
//! Sub-second precision is intentionally dropped: every `Duration` carried by
//! [`AutoReindexConfig`](crate::collection::auto_reindex::AutoReindexConfig)
//! is a cooldown measured in whole seconds (default: one hour).
//!
//! # Usage
//!
//! ```ignore
//! use crate::collection::config_serde::duration_secs;
//!
//! #[derive(serde::Serialize, serde::Deserialize)]
//! struct Config {
//!     #[serde(with = "duration_secs")]
//!     cooldown: std::time::Duration,
//! }
//! ```

/// Serde module persisting a [`Duration`](std::time::Duration) as `u64` seconds.
///
/// Use via `#[serde(with = "crate::collection::config_serde::duration_secs")]`.
pub mod duration_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    /// Serializes the duration as its whole-seconds count.
    ///
    /// # Errors
    ///
    /// Returns `S::Error` if the serializer rejects the `u64` value.
    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }

    /// Deserializes a whole-seconds count into a [`Duration`].
    ///
    /// # Errors
    ///
    /// Returns `D::Error` if the input is not a non-negative integer.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod config_serde_tests {
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
}
