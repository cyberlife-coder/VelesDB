use super::{
    evict_min_idle_from_raw, keep_alive_from_raw, DEFAULT_HTTP_EVICT_MIN_IDLE,
    DEFAULT_HTTP_KEEP_ALIVE,
};
use std::time::Duration;

#[test]
fn eviction_floor_defaults_to_five_minutes_and_honours_a_valid_value() {
    assert_eq!(
        evict_min_idle_from_raw(None, DEFAULT_HTTP_KEEP_ALIVE),
        DEFAULT_HTTP_EVICT_MIN_IDLE
    );
    assert_eq!(DEFAULT_HTTP_EVICT_MIN_IDLE, Duration::from_secs(300));
    assert_eq!(
        evict_min_idle_from_raw(Some(" 900 "), DEFAULT_HTTP_KEEP_ALIVE),
        Duration::from_secs(900)
    );
}

#[test]
fn eviction_floor_zero_or_nonsense_falls_back_instead_of_disabling_the_guard() {
    // Zero would let any local process evict every idle client at the cap.
    for raw in ["0", "", "soon", "-30", "1.5"] {
        assert_eq!(
            evict_min_idle_from_raw(Some(raw), DEFAULT_HTTP_KEEP_ALIVE),
            DEFAULT_HTTP_EVICT_MIN_IDLE,
            "raw value {raw:?}"
        );
    }
}

#[test]
fn eviction_floor_never_exceeds_the_keep_alive() {
    let keep_alive = Duration::from_secs(120);
    assert_eq!(
        evict_min_idle_from_raw(Some("7200"), keep_alive),
        keep_alive,
        "a floor above the keep-alive can never be reached"
    );
    assert_eq!(
        evict_min_idle_from_raw(None, keep_alive),
        keep_alive,
        "the default is bounded by a shorter keep-alive too"
    );
}

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
