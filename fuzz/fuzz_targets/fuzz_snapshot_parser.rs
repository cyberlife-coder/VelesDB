//! Fuzz target for snapshot parser.
//!
//! # P1 Audit: Snapshot Security
//!
//! The `load_snapshot` function reads binary data and makes allocations based
//! on user-controlled values. This is a potential DoS attack vector:
//! - Malicious snapshot could claim huge entry_count → OOM
//! - Corrupted data could cause panics or undefined behavior
//!
//! This fuzzer tests robustness against malformed snapshot files.

#![no_main]

use libfuzzer_sys::fuzz_target;

/// Maximum allocation size to prevent OOM during fuzzing (100MB)
const MAX_ALLOC_SIZE: usize = 100 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    // Try to parse the data as a snapshot
    // The parser should handle any malformed input gracefully
    let _ = parse_snapshot_safe(data);
});

/// Safe snapshot parser that limits allocations.
///
/// This mirrors the logic in `LogPayloadStorage::load_snapshot` but with
/// additional safety bounds to prevent OOM during fuzzing.
fn parse_snapshot_safe(data: &[u8]) -> Result<(), &'static str> {
    // Snapshot format:
    // [Magic: "VSNP" 4 bytes]
    // [Version: 1 byte]
    // [WAL position: 8 bytes]
    // [Entry count: 8 bytes]
    // [Entries: (id: u64, offset: u64) × N]
    // [CRC32: 4 bytes]

    const SNAPSHOT_MAGIC: &[u8; 4] = b"VSNP";
    const SNAPSHOT_VERSION: u8 = 1;
    const HEADER_SIZE: usize = 21; // magic(4) + version(1) + wal_pos(8) + count(8)
    const ENTRY_SIZE: usize = 16; // id(8) + offset(8)
    const CRC_SIZE: usize = 4;

    // Minimum size check
    if data.len() < HEADER_SIZE + CRC_SIZE {
        return Err("Snapshot too small");
    }

    // Validate magic
    if &data[0..4] != SNAPSHOT_MAGIC {
        return Err("Invalid magic");
    }

    // Validate version
    if data[4] != SNAPSHOT_VERSION {
        return Err("Unsupported version");
    }

    // Read WAL position (not validated, just parsed)
    let _wal_pos = u64::from_le_bytes(
        data[5..13]
            .try_into()
            .map_err(|_| "Invalid WAL position")?,
    );

    // Read entry count - THIS IS THE CRITICAL CHECK
    let entry_count = u64::from_le_bytes(
        data[13..21]
            .try_into()
            .map_err(|_| "Invalid entry count")?,
    ) as usize;

    // P1 AUDIT: Validate entry_count against actual data size BEFORE allocation
    // This prevents DoS via malicious entry_count claiming millions of entries
    let expected_size = HEADER_SIZE + entry_count.saturating_mul(ENTRY_SIZE) + CRC_SIZE;

    if expected_size > data.len() {
        return Err("Entry count exceeds data size");
    }

    if data.len() != expected_size {
        return Err("Size mismatch");
    }

    // P1 AUDIT: Limit allocation size to prevent OOM
    let alloc_size = entry_count * std::mem::size_of::<(u64, u64)>();
    if alloc_size > MAX_ALLOC_SIZE {
        return Err("Allocation too large");
    }

    // Validate CRC
    let stored_crc = u32::from_le_bytes(
        data[data.len() - 4..]
            .try_into()
            .map_err(|_| "Invalid CRC")?,
    );
    let computed_crc = crc32_hash(&data[..data.len() - 4]);

    if stored_crc != computed_crc {
        return Err("CRC mismatch");
    }

    // Parse entries (this is where allocation happens)
    let mut entries = Vec::with_capacity(entry_count);
    let entries_start = HEADER_SIZE;

    for i in 0..entry_count {
        let offset = entries_start + i * ENTRY_SIZE;

        if offset + ENTRY_SIZE > data.len() - CRC_SIZE {
            return Err("Entry overflow");
        }

        let id = u64::from_le_bytes(
            data[offset..offset + 8]
                .try_into()
                .map_err(|_| "Invalid entry ID")?,
        );
        let wal_offset = u64::from_le_bytes(
            data[offset + 8..offset + 16]
                .try_into()
                .map_err(|_| "Invalid entry offset")?,
        );

        entries.push((id, wal_offset));
    }

    // Success - snapshot was parsed without issues
    let _ = entries; // Use the result to prevent optimization
    Ok(())
}

/// Simple CRC32 implementation (IEEE 802.3 polynomial).
#[allow(clippy::cast_possible_truncation)]
fn crc32_hash(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut j = 0;
            while j < 8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB8_8320;
                } else {
                    crc >>= 1;
                }
                j += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };

    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}
