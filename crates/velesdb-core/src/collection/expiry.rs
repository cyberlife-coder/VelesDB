//! Durable point-TTL expiry predicate shared by every read surface.
//!
//! A point whose payload carries the reserved `_veles_expires_at` key (epoch
//! seconds) is expired once that instant is reached (`exp <= now`, so a TTL
//! of 0 expires immediately). Expired points are skipped on all read paths
//! (search/get/scroll/query/MATCH); their storage is reclaimed lazily (agent
//! `auto_expire` sweep). Internal raw reads (`Collection::get_raw`) bypass
//! this filter so TTL rebuild and snapshots still see unswept points.

/// Reserved payload key carrying the durable expiry timestamp (epoch seconds).
///
/// A point whose payload carries this key (value: `u64` epoch seconds) is
/// considered expired once `expires_at <= now`. This key is namespaced with a
/// `_veles_` prefix so it never collides with user-defined payload fields.
/// External crates (e.g. the REST server) should reference this constant
/// rather than hardcoding the string literal.
pub const EXPIRES_AT_KEY: &str = "_veles_expires_at";

/// Returns the current Unix time in seconds (0 if the clock predates epoch).
pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Returns `true` when the payload carries a [`EXPIRES_AT_KEY`] epoch-seconds
/// field whose value is `<= now_secs` (a TTL of 0 expires immediately).
pub(crate) fn is_payload_expired(payload: Option<&serde_json::Value>, now_secs: u64) -> bool {
    let Some(serde_json::Value::Object(map)) = payload else {
        return false;
    };
    map.get(EXPIRES_AT_KEY)
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|exp| exp <= now_secs)
}
