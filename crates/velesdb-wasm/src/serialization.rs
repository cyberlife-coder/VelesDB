//! Binary serialization for VectorStore.
//!
//! Format v1:
//! - Header: "VELS" (4B) + version (1B) + dimension (4B) + metric (1B) + count (8B)
//! - Data: [id (8B) + vector (dimension * 4B)] * count

use crate::distance::DistanceMetric;

/// Serializes vector store data to binary format.
pub fn export_to_bytes(
    ids: &[u64],
    data: &[f32],
    dimension: usize,
    metric: &DistanceMetric,
) -> Vec<u8> {
    let count = ids.len();
    let vector_size = 8 + dimension * 4;
    let total_size = 18 + count * vector_size;
    let mut bytes = Vec::with_capacity(total_size);

    // Header
    bytes.extend_from_slice(b"VELS");
    bytes.push(1); // version

    #[allow(clippy::cast_possible_truncation)]
    let dim_u32 = dimension as u32;
    bytes.extend_from_slice(&dim_u32.to_le_bytes());

    let metric_byte = match metric {
        DistanceMetric::Cosine => 0u8,
        DistanceMetric::Euclidean => 1u8,
        DistanceMetric::DotProduct => 2u8,
        DistanceMetric::Hamming => 3u8,
        DistanceMetric::Jaccard => 4u8,
    };
    bytes.push(metric_byte);

    #[allow(clippy::cast_possible_truncation)]
    let count_u64 = count as u64;
    bytes.extend_from_slice(&count_u64.to_le_bytes());

    // Data
    for (idx, &id) in ids.iter().enumerate() {
        bytes.extend_from_slice(&id.to_le_bytes());
        let start = idx * dimension;
        let data_slice = &data[start..start + dimension];
        // SAFETY: f32 slice to u8 slice, same memory layout
        let data_bytes: &[u8] =
            unsafe { core::slice::from_raw_parts(data_slice.as_ptr().cast::<u8>(), dimension * 4) };
        bytes.extend_from_slice(data_bytes);
    }

    bytes
}

/// Result of parsing binary data header.
pub struct ParsedHeader {
    pub dimension: usize,
    pub metric: DistanceMetric,
    pub count: usize,
}

/// Parses and validates binary data header.
pub fn parse_header(bytes: &[u8]) -> Result<ParsedHeader, &'static str> {
    if bytes.len() < 18 {
        return Err("Invalid data: too short");
    }
    if &bytes[0..4] != b"VELS" {
        return Err("Invalid data: wrong magic number");
    }
    if bytes[4] != 1 {
        return Err("Unsupported version");
    }

    let dimension = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
    let metric = match bytes[9] {
        0 => DistanceMetric::Cosine,
        1 => DistanceMetric::Euclidean,
        2 => DistanceMetric::DotProduct,
        3 => DistanceMetric::Hamming,
        4 => DistanceMetric::Jaccard,
        _ => return Err("Invalid metric byte"),
    };

    #[allow(clippy::cast_possible_truncation)]
    let count = u64::from_le_bytes([
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
    ]) as usize;

    let vector_size = 8 + dimension * 4;
    let expected_size = 18 + count * vector_size;
    if bytes.len() < expected_size {
        return Err("Invalid data: size mismatch");
    }

    Ok(ParsedHeader {
        dimension,
        metric,
        count,
    })
}

/// Imports IDs and data from binary format.
pub fn import_data(bytes: &[u8], header: &ParsedHeader) -> (Vec<u64>, Vec<f32>) {
    let count = header.count;
    let dimension = header.dimension;
    let data_bytes_len = dimension * 4;

    let mut ids = Vec::with_capacity(count);
    let total_floats = count * dimension;
    let mut data = vec![0.0_f32; total_floats];

    // Read IDs
    let mut offset = 18;
    for _ in 0..count {
        let id = u64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        ids.push(id);
        offset += 8 + data_bytes_len;
    }

    // Bulk copy vector data
    // SAFETY: f32 and [u8; 4] have same size, WASM is little-endian
    let data_as_bytes: &mut [u8] = unsafe {
        core::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), total_floats * 4)
    };

    offset = 18 + 8;
    for i in 0..count {
        let dest_start = i * dimension * 4;
        let dest_end = dest_start + data_bytes_len;
        data_as_bytes[dest_start..dest_end]
            .copy_from_slice(&bytes[offset..offset + data_bytes_len]);
        offset += 8 + data_bytes_len;
    }

    (ids, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let ids = vec![1, 2, 3];
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let dimension = 2;
        let metric = DistanceMetric::Cosine;

        let bytes = export_to_bytes(&ids, &data, dimension, &metric);
        let header = parse_header(&bytes).unwrap();
        let (imported_ids, imported_data) = import_data(&bytes, &header);

        assert_eq!(ids, imported_ids);
        assert_eq!(data, imported_data);
    }
}
