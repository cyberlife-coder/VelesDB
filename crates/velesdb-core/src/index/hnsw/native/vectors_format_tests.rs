//! On-disk format tests for `{basename}.vectors`.
//!
//! The payload moved from byte 16 to byte 4096 in v2 (#2173), so two things
//! need holding down: a file written before that change must still load, and
//! a file written after it must actually be page-aligned. Neither is covered
//! by the save/load round-trip tests, which only ever see the version the
//! current binary writes.

use super::super::distance::CachedSimdDistance;
use super::NativeHnsw;
use crate::distance::DistanceMetric;
use std::io::Write;
use tempfile::tempdir;

type H = NativeHnsw<CachedSimdDistance>;

/// Writes a v1 `.vectors` file by hand: the payload starts at byte 16, with
/// no padding. This is the layout of every index persisted before v2.
fn write_v1_vectors_file(path: &std::path::Path, vectors: &[Vec<f32>]) {
    let dimension = vectors[0].len();
    let mut file = std::fs::File::create(path).expect("test: create");
    file.write_all(&1u32.to_le_bytes()).expect("test: version");
    file.write_all(&(vectors.len() as u64).to_le_bytes())
        .expect("test: count");
    file.write_all(&(dimension as u32).to_le_bytes())
        .expect("test: dimension");
    for vector in vectors {
        for value in vector {
            file.write_all(&value.to_le_bytes()).expect("test: payload");
        }
    }
}

/// A `.vectors` file written before v2 must still load, byte for byte.
///
/// Without this, every database persisted by an earlier binary fails to open
/// with `Unsupported version` — the loudest possible regression, and one the
/// round-trip tests cannot see because they only exercise the version the
/// current binary writes.
#[test]
fn v1_vectors_file_still_loads() {
    // Arrange
    let dir = tempdir().expect("test: tempdir");
    let path = dir.path().join("legacy.vectors");
    let vectors = vec![vec![1.0f32, 2.0, 3.0, 4.0], vec![-5.5f32, 0.0, 7.25, 8.0]];
    write_v1_vectors_file(&path, &vectors);

    // Act
    let (storage, count) = H::load_vectors_file(&path, None).expect("test: load v1");

    // Assert
    assert_eq!(count, 2);
    let storage = storage.expect("test: v1 file must yield storage");
    for (i, expected) in vectors.iter().enumerate() {
        assert_eq!(
            storage.get(i).expect("test: vector present"),
            expected.as_slice(),
            "v1 vector {i} did not survive the load"
        );
    }
}

/// A freshly dumped `.vectors` file declares v2 and starts its payload on a
/// page boundary, with the reserved gap zeroed.
///
/// The alignment is the whole reason v2 exists: the arena hands out `&[f32]`
/// built with `slice::from_raw_parts`, so a payload that did not start aligned
/// could not be mapped as one. The zero-fill is what lets a later version tell
/// an unset reserved field from a set one.
#[test]
fn dumped_vectors_file_is_v2_and_page_aligned() {
    // Arrange
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);
    for i in 0..3 {
        hnsw.insert(&[i as f32; 4]).expect("test: insert");
    }
    let dir = tempdir().expect("test: tempdir");
    hnsw.file_dump(dir.path(), "aligned").expect("test: dump");

    // Act
    let bytes = std::fs::read(dir.path().join("aligned.vectors")).expect("test: read back");

    // Assert
    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().expect("test: version bytes")),
        2,
        "the dump must declare v2"
    );
    assert!(
        bytes[16..4096].iter().all(|&b| b == 0),
        "the reserved gap between header and payload must be zero-filled"
    );
    assert_eq!(
        bytes.len(),
        4096 + 3 * 4 * std::mem::size_of::<f32>(),
        "the payload must start at 4096, with nothing between it and the header"
    );
}

/// The round trip through the current writer and reader agrees on the offset.
#[test]
fn v2_dump_reloads_its_own_vectors() {
    // Arrange
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);
    for i in 0..5 {
        hnsw.insert(&[i as f32 * 1.5; 4]).expect("test: insert");
    }
    let dir = tempdir().expect("test: tempdir");
    hnsw.file_dump(dir.path(), "roundtrip").expect("test: dump");

    // Act
    let (storage, count) =
        H::load_vectors_file(&dir.path().join("roundtrip.vectors"), None).expect("test: load v2");

    // Assert: the values, not just the count — a reader that skipped to the
    // wrong offset would hand back the zeroed reserved gap and still report
    // five vectors.
    assert_eq!(count, 5);
    let storage = storage.expect("test: v2 file must yield storage");
    for i in 0..5 {
        let expected = [i as f32 * 1.5; 4];
        assert_eq!(
            storage.get(i).expect("test: vector present"),
            expected.as_slice(),
            "v2 vector {i} did not survive the round trip"
        );
    }
}
