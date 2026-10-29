//! Reconstructs f32 vectors from the quantized buffers of a `VELS` v2 blob.
//!
//! Before format v3, a `VectorStore` in `sq8`/`pq`/`rabitq` mode kept only a
//! per-vector SQ8 encoding (`data_sq8` codes plus a `min` and an encode scale
//! of `255 / (max - min)` per vector), and `binary` kept only packed sign
//! bits. Those stores searched over the reconstruction, so loading one
//! rebuilds that same reconstruction as the f32 vectors every mode now keeps.
//! The original f32 values are not recoverable from the codes.
//!
//! Decoding goes through core's own types — `QuantizedVector::to_f32` and
//! `BinaryQuantizedVector`'s bit unpacking — rather than a local copy. The
//! old SQ8 decode was `q / scale + min`, core's is `q * (range / 255) + min`,
//! so a reloaded legacy vector can differ from what the old build scored by
//! float rounding (a few ULP), never by a quantization step.

use velesdb_core::{BinaryQuantizedVector, QuantizationCodec, QuantizedVector};

use crate::StorageMode;

/// The quantized buffers of a v2 blob, as read from disk.
pub(super) struct LegacyBuffers {
    pub data: Vec<f32>,
    pub data_sq8: Vec<u8>,
    pub data_binary: Vec<u8>,
    pub sq8_mins: Vec<f32>,
    pub sq8_scales: Vec<f32>,
}

/// Returns the `count * dimension` f32 vectors a v2 store in `mode` held.
///
/// # Errors
/// Returns an error when a buffer's length does not match `count` and
/// `dimension`, instead of indexing out of bounds on a corrupt blob.
pub(super) fn reconstruct_vectors(
    mode: StorageMode,
    count: usize,
    dimension: usize,
    buffers: LegacyBuffers,
) -> Result<Vec<f32>, String> {
    match mode {
        StorageMode::Full => Ok(buffers.data),
        StorageMode::SQ8 | StorageMode::ProductQuantization | StorageMode::RaBitQ => {
            decode_sq8(count, dimension, &buffers)
        }
        StorageMode::Binary => decode_binary(count, dimension, &buffers.data_binary),
    }
}

fn expect_len(name: &str, actual: usize, rows: usize, per_row: usize) -> Result<(), String> {
    let expected = rows
        .checked_mul(per_row)
        .ok_or_else(|| format!("Invalid data: {name} size overflows usize"))?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "Invalid data: {name} holds {actual} entries, expected {expected}"
        ))
    }
}

fn decode_sq8(count: usize, dimension: usize, buffers: &LegacyBuffers) -> Result<Vec<f32>, String> {
    expect_len("data_sq8", buffers.data_sq8.len(), count, dimension)?;
    expect_len("sq8_mins", buffers.sq8_mins.len(), count, 1)?;
    expect_len("sq8_scales", buffers.sq8_scales.len(), count, 1)?;
    let mut out = Vec::with_capacity(buffers.data_sq8.len());
    for (row, (&min, &scale)) in buffers.sq8_mins.iter().zip(&buffers.sq8_scales).enumerate() {
        let codes = &buffers.data_sq8[row * dimension..(row + 1) * dimension];
        let quantized = QuantizedVector {
            data: codes.to_vec(),
            min,
            max: legacy_max(min, scale),
        };
        out.extend(quantized.to_f32());
    }
    Ok(out)
}

/// The `max` core's `QuantizedVector` needs, from the v2 per-vector `scale`.
///
/// v2 wrote `255 / range` for a normal vector and two sentinels: `0.0` for a
/// degenerate (near-constant) range, which core decodes as `min` everywhere
/// when `max == min`, and `NaN` for a non-finite range, which core decodes as
/// `NaN` everywhere when `max` is `NaN` — the values the v2 build restored.
fn legacy_max(min: f32, scale: f32) -> f32 {
    if scale == 0.0 {
        min
    } else {
        min + 255.0 / scale
    }
}

fn decode_binary(count: usize, dimension: usize, packed: &[u8]) -> Result<Vec<f32>, String> {
    let bytes_per_row = dimension.div_ceil(8);
    expect_len("data_binary", packed.len(), count, bytes_per_row)?;
    let header = u32::try_from(dimension)
        .map_err(|_| "Invalid data: dimension exceeds u32".to_string())?
        .to_le_bytes();
    let mut out = Vec::with_capacity(count.saturating_mul(dimension));
    for row in packed.chunks_exact(bytes_per_row.max(1)).take(count) {
        let encoded = [header.as_slice(), row].concat();
        let bits = BinaryQuantizedVector::from_bytes(&encoded)
            .map_err(|e| format!("Invalid data: binary row: {e}"))?;
        out.extend(
            bits.get_bits()
                .into_iter()
                .map(|bit| if bit { 1.0 } else { 0.0 }),
        );
    }
    Ok(out)
}
