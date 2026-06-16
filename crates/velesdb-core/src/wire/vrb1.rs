//! VRB1 ("Veles Raw Bulk v1") binary wire codec.
//!
//! A length-prefixed, tightly-packed binary encoding of `(id, vector)` pairs
//! for zero-copy bulk upsert, avoiding the per-point JSON overhead of the
//! object endpoints. Payloads are not carried on this path.
//!
//! This is the single shared codec used by both the server raw-bulk handler
//! and the CLI `.bin` importer — neither should re-parse the format itself.
//!
//! # Wire format (little-endian)
//!
//! ```text
//! offset  size                     field
//! ------  -----------------------  --------------------------------------
//! 0       4                        magic  = b"VRB1"  (Veles Raw Bulk v1)
//! 4       4                        count  : u32      (number of points)
//! 8       4                        dim    : u32      (vector dimension)
//! 12      1                        id_width : u8     (must be 8 → u64)
//! 13      3                        reserved (must be 0) — header is 16 bytes
//! 16      count * 8                ids    : [u64; count]
//! 16+8c   count * dim * 4          vectors: [f32; count * dim] (row-major)
//! ```
//!
//! The total length must be **exactly** `16 + count * 8 + count * dim * 4`
//! bytes; any mismatch is an error. The encoding is deterministic: a given
//! batch always serialises to the same bytes (see [`encode`] / [`decode`]).

use std::fmt;

/// 4-byte magic prefix identifying the v1 raw-bulk wire format.
const MAGIC: &[u8; 4] = b"VRB1";

/// Fixed header length: `magic`(4) + `count`(4) + `dim`(4) + `id_width`(1) + `reserved`(3).
const HEADER_LEN: usize = 16;

/// The only supported id width: `u64` ids are 8 bytes each.
const ID_WIDTH: u8 = 8;

/// A decoded VRB1 batch: owned `ids`, a flat row-major `vectors` buffer, and
/// the declared `dimension`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawBulk {
    /// Point ids, in batch order.
    pub ids: Vec<u64>,
    /// Flat `f32` buffer of shape `(ids.len(), dimension)`, row-major.
    pub vectors: Vec<f32>,
    /// Declared vector dimension.
    pub dimension: usize,
}

/// Errors produced while decoding a VRB1 body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VrbError {
    /// Body shorter than the fixed 16-byte header.
    TooShort {
        /// Actual body length in bytes.
        got: usize,
    },
    /// First four bytes are not `b"VRB1"`.
    BadMagic,
    /// `id_width` byte is not the supported value (8).
    BadIdWidth(u8),
    /// One or more reserved header bytes were non-zero.
    ReservedNotZero,
    /// Arithmetic overflow while computing the expected body length.
    Overflow,
    /// Body length does not match the length implied by `count`/`dim`.
    LengthMismatch {
        /// Actual body length in bytes.
        got: usize,
        /// Length implied by the header.
        expected: usize,
    },
}

impl fmt::Display for VrbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { got } => {
                write!(f, "body too short: {got} bytes (header needs {HEADER_LEN})")
            }
            Self::BadMagic => write!(f, "bad magic: expected b\"VRB1\""),
            Self::BadIdWidth(w) => {
                write!(
                    f,
                    "unsupported id_width {w}: only {ID_WIDTH} (u64) is supported"
                )
            }
            Self::ReservedNotZero => write!(f, "reserved header bytes must be zero"),
            Self::Overflow => write!(f, "overflow computing body length"),
            Self::LengthMismatch { got, expected } => {
                write!(f, "body length {got} != expected {expected}")
            }
        }
    }
}

impl std::error::Error for VrbError {}

/// Parse the fixed 16-byte header, returning `(count, dim)`.
///
/// Validates the magic, the id width, and the reserved padding so a malformed
/// or wrong-version body is rejected before any allocation.
fn parse_header(body: &[u8]) -> Result<(usize, usize), VrbError> {
    if body.len() < HEADER_LEN {
        return Err(VrbError::TooShort { got: body.len() });
    }
    if &body[0..4] != MAGIC {
        return Err(VrbError::BadMagic);
    }
    let count = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
    let dim = u32::from_le_bytes([body[8], body[9], body[10], body[11]]) as usize;
    if body[12] != ID_WIDTH {
        return Err(VrbError::BadIdWidth(body[12]));
    }
    if body[13] != 0 || body[14] != 0 || body[15] != 0 {
        return Err(VrbError::ReservedNotZero);
    }
    Ok((count, dim))
}

/// Compute the expected total body length for `count` points of `dim` floats.
///
/// Returns [`VrbError::Overflow`] on arithmetic overflow rather than panicking.
fn expected_body_len(count: usize, dim: usize) -> Result<usize, VrbError> {
    let ids_bytes = count.checked_mul(8).ok_or(VrbError::Overflow)?;
    let vec_elems = count.checked_mul(dim).ok_or(VrbError::Overflow)?;
    let vec_bytes = vec_elems.checked_mul(4).ok_or(VrbError::Overflow)?;
    HEADER_LEN
        .checked_add(ids_bytes)
        .and_then(|h| h.checked_add(vec_bytes))
        .ok_or(VrbError::Overflow)
}

/// Decode the tightly-packed id section into a `Vec<u64>`.
///
/// The caller validates the exact body length first, so every 8-byte chunk is
/// in bounds; `chunks_exact` drops any trailing partial chunk safely.
fn decode_ids(body: &[u8], count: usize) -> Vec<u64> {
    let start = HEADER_LEN;
    let end = start + count * 8;
    body[start..end]
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
        .collect()
}

/// Decode the tightly-packed vector section into a flat `Vec<f32>`.
fn decode_vectors(body: &[u8], count: usize, dim: usize) -> Vec<f32> {
    let start = HEADER_LEN + count * 8;
    let end = start + count * dim * 4;
    body[start..end]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Decode a full VRB1 body into a [`RawBulk`].
///
/// Validates the header and the exact total length before decoding, so all
/// slice accesses in [`decode_ids`] / [`decode_vectors`] are in bounds.
///
/// # Errors
///
/// Returns a [`VrbError`] for a too-short body, bad magic, unsupported id
/// width, non-zero reserved bytes, length overflow, or a length mismatch.
pub fn decode(body: &[u8]) -> Result<RawBulk, VrbError> {
    let (count, dim) = parse_header(body)?;
    let expected = expected_body_len(count, dim)?;
    if body.len() != expected {
        return Err(VrbError::LengthMismatch {
            got: body.len(),
            expected,
        });
    }
    Ok(RawBulk {
        ids: decode_ids(body, count),
        vectors: decode_vectors(body, count, dim),
        dimension: dim,
    })
}

/// Encode an `(ids, vectors)` batch into the VRB1 wire format.
///
/// The inverse of [`decode`]; `vectors` is a flat row-major buffer of shape
/// `(ids.len(), dimension)`. The caller is responsible for the invariant
/// `vectors.len() == ids.len() * dimension`; a mismatch round-trips to a
/// body that [`decode`] rejects with [`VrbError::LengthMismatch`].
#[must_use]
pub fn encode(ids: &[u64], vectors: &[f32], dimension: usize) -> Vec<u8> {
    let count = ids.len();
    let mut buf = Vec::with_capacity(HEADER_LEN + count * 8 + vectors.len() * 4);
    buf.extend_from_slice(MAGIC);
    // `count`/`dimension` exceeding u32::MAX (~4.3 billion) cannot be held in
    // memory on this path, so the saturation is unreachable for any encodable
    // batch and never corrupts a representable one.
    buf.extend_from_slice(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
    buf.extend_from_slice(&u32::try_from(dimension).unwrap_or(u32::MAX).to_le_bytes());
    buf.push(ID_WIDTH);
    buf.extend_from_slice(&[0u8; 3]);
    for id in ids {
        buf.extend_from_slice(&id.to_le_bytes());
    }
    for v in vectors {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
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
}
