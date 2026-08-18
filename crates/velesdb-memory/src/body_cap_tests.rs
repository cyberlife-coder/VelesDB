use super::DEFAULT_HTTP_MAX_BODY_BYTES;
use crate::limits::{MAX_TOTAL_MEDIA_BYTES, MAX_TRANSCRIPT_BYTES};

/// The daemon's default transport cap must carry every request the core
/// itself accepts: the full published media budget plus the largest
/// single text field, with framing on top (#1746). Asserted as a
/// RELATION between the constants, not as a number — the next adjustment
/// to either side must re-face this invariant instead of a stale figure.
#[test]
fn the_default_body_cap_carries_the_full_media_budget() {
    assert!(
        DEFAULT_HTTP_MAX_BODY_BYTES >= MAX_TOTAL_MEDIA_BYTES + MAX_TRANSCRIPT_BYTES,
        "a request the core accepts (up to {MAX_TOTAL_MEDIA_BYTES} bytes of media \
         plus up to {MAX_TRANSCRIPT_BYTES} bytes of text) must not be refused by \
         the transport alone — stdio has no such cap, so a tighter HTTP default \
         makes the SAME call succeed or fail depending on how the client connected \
         (got {DEFAULT_HTTP_MAX_BODY_BYTES})"
    );
}
