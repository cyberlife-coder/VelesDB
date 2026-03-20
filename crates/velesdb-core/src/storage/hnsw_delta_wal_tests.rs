//! Tests for the HNSW delta WAL module.
//!
//! Covers round-trip serialization, CRC corruption detection,
//! truncated WAL recovery, and edge cases.

use super::hnsw_delta_wal::{HnswDelta, HnswDeltaReader, HnswDeltaWriter};
use std::io::Write;
use tempfile::TempDir;

/// Helper: builds a standard set of mixed delta entries for testing.
fn sample_deltas() -> Vec<HnswDelta> {
    vec![
        HnswDelta::AddEdge {
            from: 1,
            to: 2,
            layer: 0,
        },
        HnswDelta::RemoveEdge {
            from: 3,
            to: 4,
            layer: 1,
        },
        HnswDelta::SetEntry {
            node: 5,
            max_layer: 3,
        },
        HnswDelta::AddEdge {
            from: u32::MAX,
            to: 0,
            layer: 255,
        },
    ]
}

#[test]
fn round_trip_write_and_read() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hnsw_delta.wal");

    let deltas = sample_deltas();

    // Write
    let mut writer = HnswDeltaWriter::open(&path).unwrap();
    for d in &deltas {
        writer.append(d).unwrap();
    }
    writer.sync().unwrap();
    assert_eq!(writer.entry_count(), deltas.len() as u64);

    // Read back
    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert_eq!(recovered, deltas);
}

#[test]
fn empty_wal_returns_empty_vec() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.wal");

    // Create an empty file
    std::fs::File::create(&path).unwrap();

    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert!(recovered.is_empty());
}

#[test]
fn crc_corruption_stops_reading() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("corrupt.wal");

    // Write two entries
    let mut writer = HnswDeltaWriter::open(&path).unwrap();
    writer
        .append(&HnswDelta::AddEdge {
            from: 10,
            to: 20,
            layer: 0,
        })
        .unwrap();
    writer
        .append(&HnswDelta::AddEdge {
            from: 30,
            to: 40,
            layer: 1,
        })
        .unwrap();
    writer.sync().unwrap();

    // Corrupt a byte in the second entry (flip byte at offset 14+2 = 16)
    let mut data = std::fs::read(&path).unwrap();
    assert_eq!(data.len(), 28); // 14 + 14
    data[16] ^= 0xFF; // Corrupt a payload byte in the second entry
    std::fs::write(&path, &data).unwrap();

    // Read: should recover only the first entry
    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0],
        HnswDelta::AddEdge {
            from: 10,
            to: 20,
            layer: 0,
        }
    );
}

#[test]
fn truncated_wal_recovers_valid_prefix() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("truncated.wal");

    // Write three entries
    let mut writer = HnswDeltaWriter::open(&path).unwrap();
    writer
        .append(&HnswDelta::SetEntry {
            node: 1,
            max_layer: 2,
        })
        .unwrap();
    writer
        .append(&HnswDelta::AddEdge {
            from: 5,
            to: 6,
            layer: 0,
        })
        .unwrap();
    writer
        .append(&HnswDelta::RemoveEdge {
            from: 7,
            to: 8,
            layer: 1,
        })
        .unwrap();
    writer.sync().unwrap();

    // Truncate in the middle of the third entry (remove last 5 bytes)
    let data = std::fs::read(&path).unwrap();
    let truncated_len = data.len() - 5;
    std::fs::write(&path, &data[..truncated_len]).unwrap();

    // Read: should recover first two entries
    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert_eq!(recovered.len(), 2);
    assert_eq!(
        recovered[0],
        HnswDelta::SetEntry {
            node: 1,
            max_layer: 2,
        }
    );
    assert_eq!(
        recovered[1],
        HnswDelta::AddEdge {
            from: 5,
            to: 6,
            layer: 0,
        }
    );
}

#[test]
fn single_byte_wal_is_tolerated() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("single_byte.wal");

    // Write just one byte (a valid op code but no payload)
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&[1]).unwrap();
    f.sync_all().unwrap();
    drop(f);

    // Should return empty — the partial entry is discarded
    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert!(recovered.is_empty());
}

#[test]
fn invalid_op_code_stops_reading() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad_op.wal");

    // Write a valid entry then a garbage op code
    let mut writer = HnswDeltaWriter::open(&path).unwrap();
    writer
        .append(&HnswDelta::SetEntry {
            node: 42,
            max_layer: 1,
        })
        .unwrap();
    writer.sync().unwrap();
    drop(writer);

    // Append garbage
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(&[0xFF, 0x00, 0x00]).unwrap();
    f.sync_all().unwrap();
    drop(f);

    // Should recover the valid entry and stop at the bad op
    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0],
        HnswDelta::SetEntry {
            node: 42,
            max_layer: 1,
        }
    );
}

#[test]
fn append_to_existing_wal() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("append.wal");

    // First writer
    let mut w1 = HnswDeltaWriter::open(&path).unwrap();
    w1.append(&HnswDelta::AddEdge {
        from: 1,
        to: 2,
        layer: 0,
    })
    .unwrap();
    w1.sync().unwrap();
    drop(w1);

    // Second writer (re-opens in append mode)
    let mut w2 = HnswDeltaWriter::open(&path).unwrap();
    w2.append(&HnswDelta::SetEntry {
        node: 10,
        max_layer: 4,
    })
    .unwrap();
    w2.sync().unwrap();
    drop(w2);

    // Read both entries
    let mut reader = HnswDeltaReader::open(&path).unwrap();
    let recovered = reader.read_all().unwrap();
    assert_eq!(recovered.len(), 2);
    assert_eq!(
        recovered[0],
        HnswDelta::AddEdge {
            from: 1,
            to: 2,
            layer: 0,
        }
    );
    assert_eq!(
        recovered[1],
        HnswDelta::SetEntry {
            node: 10,
            max_layer: 4,
        }
    );
}

use std::fs::OpenOptions;
