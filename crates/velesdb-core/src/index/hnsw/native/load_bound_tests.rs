use super::super::distance::CpuDistance;
use super::NativeHnsw;

type H = NativeHnsw<CpuDistance>;

/// REGRESSION (#899 follow-up): the persisted-index LOAD bound is the file
/// length, NOT a fixed byte ceiling. A realistic large `count` whose
/// declared payload fits the actual file length is ACCEPTED — even far above
/// the old 16 GiB cap — so a genuine index always reloads.
#[test]
fn validate_vectors_file_len_accepts_large_file_backed_count() {
    const HEADER: u64 = 16;
    // ~6.8M vectors @768D ≈ 20 GiB payload — above the old 16 GiB cap.
    let dimension = 768usize;
    let count = (20u64 * 1024 * 1024 * 1024) / (dimension as u64 * 4);
    let payload = count * dimension as u64 * 4;
    let file_len = payload + HEADER; // file genuinely holds the data
    assert!(
        H::validate_vectors_file_len(count as usize, dimension, file_len, HEADER).is_ok(),
        "a file-backed large count must load, regardless of the alloc backstop"
    );
}

/// A header declaring more data than the file can hold is rejected
/// (corrupt/malicious oversized header).
#[test]
fn validate_vectors_file_len_rejects_short_file() {
    let dimension = 128usize;
    let count = 1_000_000usize;
    // File is only 100 bytes — cannot back the declared payload.
    let err = H::validate_vectors_file_len(count, dimension, 100, 16)
        .expect_err("file shorter than declared payload must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

/// An overflow-class header (count * dimension * 4 wraps u64) is rejected
/// rather than wrapping to a small accepted size.
#[test]
fn validate_vectors_file_len_rejects_overflow_header() {
    let err = H::validate_vectors_file_len(usize::MAX, usize::MAX, u64::MAX, 16)
        .expect_err("overflow-class payload must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}
