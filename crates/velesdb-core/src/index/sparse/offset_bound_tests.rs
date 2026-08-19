use super::*;

/// #897 follow-up: offset/len bound arithmetic runs in `u64`, so an offset
/// that would truncate to a small in-bounds value on a 32-bit target (here
/// `0x1_0000_0000`, which is `0` as `u32`) is still rejected against the real
/// file length, and a valid entry is accepted.
#[test]
fn build_postings_rejects_offset_past_file() {
    let idx = vec![0u8; POSTING_DISK_SIZE]; // exactly one posting
    let entry = |offset, len| TermEntry {
        term_id: 1,
        offset,
        len,
        max_weight: 1.0,
    };

    assert!(build_postings_from_idx(&idx, &[entry(0, 1)]).is_ok());
    assert!(
        build_postings_from_idx(&idx, &[entry(0x1_0000_0000, 1)]).is_err(),
        "offset past file must be rejected via the u64 bound, not 32-bit-truncated"
    );
    assert!(build_postings_from_idx(&idx, &[entry(u64::MAX, u32::MAX)]).is_err());
}
