//! Unit tests for volatile log-prefix masking.

use super::mask_volatile_prefix;

#[test]
fn test_mask_volatile_prefix_strips_iso_timestamp_with_millis_and_zulu() {
    let masked = mask_volatile_prefix("2026-07-18T10:23:45.123Z INFO canary check passed")
        .expect("an ISO timestamp must be recognized");
    assert_eq!(masked, "<TS> INFO canary check passed");
}

#[test]
fn test_mask_volatile_prefix_strips_iso_timestamp_with_comma_millis_and_offset() {
    let masked = mask_volatile_prefix("2026-07-18 10:23:45,123+02:00 WARN retrying upstream")
        .expect("a comma-millis offset timestamp must be recognized");
    assert_eq!(masked, "<TS> WARN retrying upstream");
}

#[test]
fn test_mask_volatile_prefix_strips_syslog_timestamp_single_digit_day() {
    let masked = mask_volatile_prefix("Jul  8 10:23:45 ERROR timeout connecting to shard-3")
        .expect("a syslog timestamp must be recognized");
    assert_eq!(masked, "<TS> ERROR timeout connecting to shard-3");
}

#[test]
fn test_mask_volatile_prefix_strips_bracketed_hex_counter_after_timestamp() {
    let masked = mask_volatile_prefix("2026-07-18T10:23:45Z [a1b2c3] INFO canary check passed")
        .expect("a timestamp plus bracketed hex counter must be recognized");
    assert_eq!(masked, "<TS> INFO canary check passed");
}

#[test]
fn test_mask_volatile_prefix_strips_bracketed_pid_without_leading_timestamp() {
    let masked = mask_volatile_prefix("[12345] worker restarted")
        .expect("a leading bracketed pid must be recognized on its own");
    assert_eq!(masked, "<TS> worker restarted");
}

#[test]
fn test_mask_volatile_prefix_leaves_level_tag_bracket_alone() {
    // "ERROR" contains non-hex letters (R, O), so it must not be mistaken
    // for a pid/hex counter.
    assert_eq!(mask_volatile_prefix("[ERROR] shard-3 timeout"), None);
}

#[test]
fn test_mask_volatile_prefix_returns_none_without_any_volatile_prefix() {
    assert_eq!(
        mask_volatile_prefix("canary check passed for shard-1"),
        None
    );
}
