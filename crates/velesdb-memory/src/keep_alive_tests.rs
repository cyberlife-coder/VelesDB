use super::{keep_alive_from_raw, DEFAULT_HTTP_KEEP_ALIVE};
use std::time::Duration;

#[test]
fn unset_falls_back_to_the_sixty_minute_default() {
    assert_eq!(keep_alive_from_raw(None), DEFAULT_HTTP_KEEP_ALIVE);
    assert_eq!(
        DEFAULT_HTTP_KEEP_ALIVE,
        Duration::from_secs(3600),
        "the default must stay well beyond an agent's normal silences — a CI \
         wait alone already approaches 30 minutes"
    );
}

#[test]
fn a_valid_value_is_honoured() {
    assert_eq!(
        keep_alive_from_raw(Some("900")),
        Duration::from_secs(900),
        "the timeout must be configurable, not hard-coded"
    );
    assert_eq!(
        keep_alive_from_raw(Some("  120  ")),
        Duration::from_secs(120)
    );
}

#[test]
fn unparseable_or_zero_falls_back_instead_of_bricking_the_daemon() {
    // Zero would retire every session the instant it was created, so the
    // daemon would answer 404 to every second request. Falling back is the
    // only safe reading of a nonsense value.
    assert_eq!(keep_alive_from_raw(Some("0")), DEFAULT_HTTP_KEEP_ALIVE);
    assert_eq!(keep_alive_from_raw(Some("")), DEFAULT_HTTP_KEEP_ALIVE);
    assert_eq!(keep_alive_from_raw(Some("soon")), DEFAULT_HTTP_KEEP_ALIVE);
    assert_eq!(keep_alive_from_raw(Some("-30")), DEFAULT_HTTP_KEEP_ALIVE);
    assert_eq!(keep_alive_from_raw(Some("1.5")), DEFAULT_HTTP_KEEP_ALIVE);
}
