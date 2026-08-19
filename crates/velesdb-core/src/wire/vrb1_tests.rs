use super::*;

#[test]
fn roundtrip_decode_encode() {
    let ids = [1u64, 2, 3];
    let vectors = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6];
    let body = encode(&ids, &vectors, 2);
    let raw = decode(&body).expect("valid body decodes");
    assert_eq!(raw.ids, vec![1, 2, 3]);
    assert_eq!(raw.vectors, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
    assert_eq!(raw.dimension, 2);
}

#[test]
fn encode_is_deterministic_and_pinned() {
    let ids = [7u64, 42];
    let vectors = [1.0f32, 2.0, 3.0, 4.0];
    let a = encode(&ids, &vectors, 2);
    let b = encode(&ids, &vectors, 2);
    assert_eq!(a, b, "encoding must be deterministic");
    assert_eq!(&a[0..4], b"VRB1");
    assert_eq!(&a[4..8], &2u32.to_le_bytes());
    assert_eq!(&a[8..12], &2u32.to_le_bytes());
    assert_eq!(a[12], 8);
    assert_eq!(&a[13..16], &[0, 0, 0]);
}

#[test]
fn empty_batch_roundtrips() {
    let body = encode(&[], &[], 4);
    let raw = decode(&body).expect("empty batch decodes");
    assert!(raw.ids.is_empty());
    assert!(raw.vectors.is_empty());
    assert_eq!(raw.dimension, 4);
}

#[test]
fn bad_magic_rejected() {
    let mut body = encode(&[1], &[0.0, 0.0], 2);
    body[0] = b'X';
    assert_eq!(decode(&body), Err(VrbError::BadMagic));
}

#[test]
fn short_body_rejected() {
    let body = vec![0u8; 4];
    assert_eq!(decode(&body), Err(VrbError::TooShort { got: 4 }));
}

#[test]
fn bad_id_width_rejected() {
    let mut body = encode(&[1], &[0.0, 0.0], 2);
    body[12] = 4; // u32 ids unsupported
    assert_eq!(decode(&body), Err(VrbError::BadIdWidth(4)));
}

#[test]
fn reserved_not_zero_rejected() {
    let mut body = encode(&[1], &[0.0, 0.0], 2);
    body[13] = 1;
    assert_eq!(decode(&body), Err(VrbError::ReservedNotZero));
}

#[test]
fn length_mismatch_rejected() {
    let mut body = encode(&[1, 2], &[0.0, 0.0, 0.0, 0.0], 2);
    body.pop(); // truncate one byte
    match decode(&body) {
        Err(VrbError::LengthMismatch { .. }) => {}
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

/// A `count`/`dim` pair whose declared body length overflows `usize` is
/// rejected with `Overflow`, not a panic, before any allocation. The body is
/// only the 16-byte header; `count`/`dim` are crafted directly so the
/// `count * dim * 4` product blows past `usize::MAX`.
#[test]
fn overflow_count_dim_rejected() {
    let mut body = Vec::with_capacity(HEADER_LEN);
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&u32::MAX.to_le_bytes()); // count
    body.extend_from_slice(&u32::MAX.to_le_bytes()); // dim
    body.push(ID_WIDTH);
    body.extend_from_slice(&[0u8; 3]);
    assert_eq!(decode(&body), Err(VrbError::Overflow));
}

/// Every `VrbError` variant renders a distinct, non-empty `Display` string,
/// and the type is usable as a `std::error::Error`.
#[test]
fn error_display_and_trait_cover_all_variants() {
    let cases: [VrbError; 6] = [
        VrbError::TooShort { got: 3 },
        VrbError::BadMagic,
        VrbError::BadIdWidth(4),
        VrbError::ReservedNotZero,
        VrbError::Overflow,
        VrbError::LengthMismatch {
            got: 10,
            expected: 16,
        },
    ];
    let rendered: Vec<String> = cases.iter().map(ToString::to_string).collect();
    assert!(rendered.iter().all(|s| !s.is_empty()));
    // Distinct messages per variant.
    let unique: std::collections::HashSet<&String> = rendered.iter().collect();
    assert_eq!(unique.len(), cases.len());
    // A couple of pinned substrings so a future message change is visible.
    assert!(rendered[0].contains("too short"));
    assert!(rendered[2].contains("id_width 4"));
    // Usable through the std error trait object.
    let err: &dyn std::error::Error = &cases[1];
    assert_eq!(err.to_string(), "bad magic: expected b\"VRB1\"");
}
