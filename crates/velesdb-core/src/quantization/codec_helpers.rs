//! Shared helpers for `QuantizationCodec` serialization and deserialization.
//!
//! RF-DEDUP: Both `QuantizedVector` (SQ8) and `BinaryQuantizedVector` share
//! the same serialization pattern: fixed-size header + opaque payload.
//! These helpers eliminate duplicated capacity pre-allocation, header
//! writing, minimum-length validation, and payload slicing logic.

use std::io;

/// Serializes a header and payload into a single byte buffer.
///
/// Pre-allocates exactly `header.len() + payload.len()` bytes, then
/// writes the header followed by the payload with zero reallocation.
///
/// # Examples (internal only)
///
/// ```ignore
/// let header = min.to_le_bytes().iter().chain(&max.to_le_bytes()).copied().collect::<Vec<u8>>();
/// let bytes = serialize_with_header(&header, &self.data);
/// ```
#[inline]
pub(crate) fn serialize_with_header(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(header.len() + payload.len());
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(payload);
    bytes
}

/// Validates that `bytes` has at least `header_size` bytes and splits it
/// into `(header, payload)`.
///
/// Returns `Err(InvalidData)` when the input is shorter than the required
/// header, embedding `type_name` in the error message for diagnostics.
///
/// # Errors
///
/// Returns [`io::Error`] with kind [`io::ErrorKind::InvalidData`] when
/// `bytes.len() < header_size`.
#[inline]
pub(crate) fn validate_and_split_header<'a>(
    bytes: &'a [u8],
    header_size: usize,
    type_name: &str,
) -> io::Result<(&'a [u8], &'a [u8])> {
    if bytes.len() < header_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Not enough bytes for {type_name} header"),
        ));
    }
    Ok(bytes.split_at(header_size))
}

#[cfg(test)]
#[path = "codec_helpers_tests.rs"]
mod codec_helpers_tests;
