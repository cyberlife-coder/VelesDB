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
    let (storage, count, _) = H::load_vectors_file(&path, None).expect("test: load v1");

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
    let (storage, count, _) =
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

/// A v1 file is read into a separate arena, never adopted: its payload starts
/// at byte 16, where a mapping whose data region begins at 4096 cannot.
#[test]
fn v1_vectors_file_is_not_adopted_as_the_arena() {
    // Arrange
    let dir = tempdir().expect("test: tempdir");
    let path = dir.path().join("legacy.vectors");
    write_v1_vectors_file(&path, &[vec![1.0f32, 2.0, 3.0, 4.0]]);

    // Act
    let (storage, _, _) = H::load_vectors_file(&path, None).expect("test: load v1");

    // Assert
    let storage = storage.expect("test: v1 file must yield storage");
    assert_eq!(
        storage.backing_path(),
        None,
        "a v1 payload starts at byte 16 and cannot be mapped in place"
    );
}

/// Dropping a graph whose arena **is** `.vectors` must leave the file alone.
///
/// This is the hazard the whole ownership question exists for: `ArenaHome`
/// deletes its file on drop, and it is correct to — the disposable arena is a
/// cache. Point that at the durable store and closing a collection destroys
/// it, silently, until the next open. The graph therefore carries no
/// `ArenaHome` at all when it adopted the durable file, and the assertion on
/// `backing_path` below is what makes this test guard that rather than pass
/// for the unrelated reason that nothing was mapped in the first place.
#[test]
fn dropping_a_graph_that_adopted_its_vectors_keeps_the_file() {
    // Arrange
    let dir = tempdir().expect("test: tempdir");
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);
    // Above the arena capacity floor: below it adoption is refused on purpose,
    // and this test would then guard a mapping that never happened.
    let nodes = crate::perf_optimizations::ContiguousVectors::MIN_ARENA_CAPACITY + 2;
    for i in 0..nodes {
        hnsw.insert(&[i as f32; 4]).expect("test: insert");
    }
    hnsw.file_dump(dir.path(), "durable").expect("test: dump");
    drop(hnsw);
    let vectors_path = dir.path().join("durable.vectors");

    // Act
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let reloaded = H::file_load(dir.path(), "durable", engine).expect("test: load");
    assert_eq!(
        reloaded
            .vectors
            .read()
            .as_ref()
            .and_then(crate::perf_optimizations::ContiguousVectors::backing_path),
        Some(vectors_path.as_path()),
        "the reload must have adopted the durable file, or this test guards nothing"
    );
    drop(reloaded);

    // Assert
    assert!(
        vectors_path.exists(),
        "dropping a graph must never delete the durable vector store"
    );
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let again = H::file_load(dir.path(), "durable", engine).expect("test: reload after drop");
    assert_eq!(again.len(), nodes, "the vectors must survive the drop");
}

/// Saving a graph that adopted its own `.vectors` keeps every value.
///
/// The old dump path opened the file with `File::create`, which truncates.
/// Against a live mapping of that same file, that is not a slow path — the
/// pages the arena still points at stop existing. There is no clean red for
/// it, because the failure is a SIGBUS rather than an assertion, which is
/// exactly why the dump branches instead of relying on a test to notice.
#[test]
fn saving_an_adopted_graph_preserves_its_vectors() {
    // Arrange
    let dir = tempdir().expect("test: tempdir");
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);
    let nodes = crate::perf_optimizations::ContiguousVectors::MIN_ARENA_CAPACITY + 2;
    for i in 0..nodes {
        hnsw.insert(&[i as f32 * 2.5; 4]).expect("test: insert");
    }
    hnsw.file_dump(dir.path(), "resave")
        .expect("test: first dump");
    drop(hnsw);

    // Act: reload (adopts the file), then save again through the mapping
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let reloaded = H::file_load(dir.path(), "resave", engine).expect("test: load");
    // The subject here is the dump path an ADOPTED arena takes; without the
    // adoption this would quietly exercise the ordinary copy path instead.
    assert!(
        reloaded
            .vectors
            .read()
            .as_ref()
            .and_then(crate::perf_optimizations::ContiguousVectors::backing_path)
            .is_some(),
        "the reload must have adopted the durable file"
    );
    reloaded
        .file_dump(dir.path(), "resave")
        .expect("test: second dump");
    drop(reloaded);

    // Assert
    let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
    let final_load = H::file_load(dir.path(), "resave", engine).expect("test: final load");
    // The guard is scoped to the read: an assertion that fires while holding a
    // lock takes the panic and the lock down together, which is a worse
    // failure than the one being reported.
    let stored: Vec<Vec<f32>> = {
        let guard = final_load.vectors.read();
        let storage = guard.as_ref().expect("test: storage present");
        let values = (0..storage.len())
            .map(|i| storage.get(i).expect("test: vector present").to_vec())
            .collect();
        drop(guard);
        values
    };

    assert_eq!(stored.len(), nodes);
    for (i, value) in stored.iter().enumerate() {
        let expected = [i as f32 * 2.5; 4];
        assert_eq!(
            value.as_slice(),
            expected.as_slice(),
            "vector {i} did not survive a save through its own mapping"
        );
    }
}

/// Opening a collection must never write to it — on either side of the arena's
/// capacity floor.
///
/// Below the floor an adopted arena would be sized up to `MIN_ARENA_CAPACITY`
/// and the file extended with it, so adoption is refused there and the copy
/// path runs instead. This is not a tidiness point: `velesdb-memory`'s
/// migration resume proves the source store unchanged by hashing these files,
/// and twenty-one of its tests failed on exactly this when adoption was
/// unconditional. The counts are derived from the constant rather than written
/// out, so moving the floor moves the test with it.
#[test]
fn opening_a_collection_never_writes_to_its_vectors() {
    use crate::perf_optimizations::ContiguousVectors;
    const FLOOR: usize = ContiguousVectors::MIN_ARENA_CAPACITY;

    for (nodes, expect_adopted) in [(FLOOR - 1, false), (FLOOR + 4, true)] {
        // Arrange
        let dir = tempdir().expect("test: tempdir");
        let name = format!("open{nodes}");
        let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);
        for i in 0..nodes {
            hnsw.insert(&[i as f32 + 0.3; 4]).expect("test: insert");
        }
        hnsw.file_dump(dir.path(), &name).expect("test: dump");
        drop(hnsw);
        let vectors_path = dir.path().join(format!("{name}.vectors"));
        let before = std::fs::read(&vectors_path).expect("test: read before");

        // Act
        let engine = CachedSimdDistance::new(DistanceMetric::Euclidean, 4);
        let loaded = H::file_load(dir.path(), &name, engine).expect("test: load");
        let adopted = loaded
            .vectors
            .read()
            .as_ref()
            .and_then(ContiguousVectors::backing_path)
            .is_some();
        drop(loaded);

        // Assert
        assert_eq!(
            adopted, expect_adopted,
            "{nodes} vectors: adoption must follow the capacity floor, or this \
             test is measuring the wrong thing"
        );
        let after = std::fs::read(&vectors_path).expect("test: read after");
        assert!(
            before == after,
            "{nodes} vectors: opening a collection wrote to its .vectors \
             ({} bytes before, {} after)",
            before.len(),
            after.len()
        );
    }
}

/// Only a pre-normalized cosine engine declares its payload unit-norm (#2246).
#[test]
fn only_a_prenormalized_cosine_dump_declares_its_payload_unit_norm() {
    let flag_at = usize::try_from(super::VECTORS_HEADER_BYTES).expect("test: offset fits usize");
    for (engine, expected) in [
        (
            CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, 4),
            super::VECTORS_FLAG_UNIT_NORM,
        ),
        (CachedSimdDistance::new(DistanceMetric::Cosine, 4), 0),
        (
            CachedSimdDistance::new_prenormalized(DistanceMetric::Euclidean, 4),
            0,
        ),
    ] {
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);
        for i in 0..3 {
            hnsw.insert(&[i as f32 + 1.0, 0.5, 0.25, 2.0])
                .expect("test: insert");
        }
        let dir = tempdir().expect("test: tempdir");
        hnsw.file_dump(dir.path(), "flag").expect("test: dump");
        let bytes = std::fs::read(dir.path().join("flag.vectors")).expect("test: read back");
        assert_eq!(bytes[flag_at], expected);
        let payload =
            usize::try_from(super::VECTORS_V2_DATA_OFFSET).expect("test: offset fits usize");
        assert!(
            bytes[flag_at + 1..payload].iter().all(|&b| b == 0),
            "the rest of the padding stays reserved and zero-filled"
        );
    }
}

/// A payload declared unit-norm is left as stored on load; one without the
/// flag is still normalized (#2246).
///
/// Both halves are needed. The first is the point: on an adopted arena that
/// check read every page of the mapping inside `load`. The second is the
/// control — a file written before the flag existed keeps the legacy
/// normalization, or a raw vector would reach the dot-product kernel.
#[test]
fn the_unit_norm_flag_is_what_skips_the_load_time_normalization() {
    const DIM: usize = 4;
    let flag_at = usize::try_from(super::VECTORS_HEADER_BYTES).expect("test: offset fits usize");
    let payload = usize::try_from(super::VECTORS_V2_DATA_OFFSET).expect("test: offset fits usize");
    for (flagged, expected_norm) in [(true, 3.0_f32), (false, 1.0_f32)] {
        let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, DIM);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);
        for i in 0..3 {
            hnsw.insert(&[i as f32 + 1.0, 0.5, 0.25, 2.0])
                .expect("test: insert");
        }
        let dir = tempdir().expect("test: tempdir");
        hnsw.file_dump(dir.path(), "norm").expect("test: dump");
        drop(hnsw);

        // Scale the first stored (unit) vector by 3, and clear the flag on
        // the control arm, as a file written before the flag would hold it.
        let path = dir.path().join("norm.vectors");
        let mut bytes = std::fs::read(&path).expect("test: read back");
        let width = std::mem::size_of::<f32>();
        for j in 0..DIM {
            let at = payload + j * width;
            let value = f32::from_le_bytes(bytes[at..at + width].try_into().expect("test: f32"));
            bytes[at..at + width].copy_from_slice(&(value * 3.0).to_le_bytes());
        }
        if !flagged {
            bytes[flag_at] = 0;
        }
        std::fs::write(&path, &bytes).expect("test: write back");

        let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, DIM);
        let loaded = H::file_load(dir.path(), "norm", engine).expect("test: load");
        let norm = crate::simd_native::norm_native(
            loaded
                .vectors
                .read()
                .as_ref()
                .and_then(|vectors| vectors.get(0))
                .expect("test: first vector"),
        );
        assert!(
            (norm - expected_norm).abs() < 1e-4,
            "flagged={flagged}: expected norm {expected_norm}, got {norm}"
        );
    }
}

/// Opening never writes the durable file — not even for a payload that needs
/// the legacy normalization (#2246).
///
/// Once adopted, the arena IS `.vectors`, and the load-time normalization ran
/// on it in place: an open that wrote the store, which `velesdb-memory`'s
/// migration resume would read as corruption. An unflagged payload with a
/// vector off the unit sphere is now copied to the heap first. The second arm
/// is the control: an unflagged payload already on the sphere stays adopted,
/// so the copy is not simply taken every time.
#[cfg(feature = "persistence")]
#[test]
fn a_legacy_payload_is_normalized_in_a_copy_never_in_the_file() {
    use crate::perf_optimizations::ContiguousVectors;
    const DIM: usize = 4;
    let nodes = ContiguousVectors::MIN_ARENA_CAPACITY + 2;
    let flag_at = usize::try_from(super::VECTORS_HEADER_BYTES).expect("test: offset fits usize");
    let payload = usize::try_from(super::VECTORS_V2_DATA_OFFSET).expect("test: offset fits usize");
    for (off_sphere, expect_adopted) in [(true, false), (false, true)] {
        let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, DIM);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);
        for i in 0..nodes {
            hnsw.insert(&[i as f32 + 1.0, 0.5, 0.25, 2.0])
                .expect("test: insert");
        }
        let dir = tempdir().expect("test: tempdir");
        hnsw.file_dump(dir.path(), "legacy").expect("test: dump");
        drop(hnsw);

        // A file written before the flag existed: flag clear and, on the
        // first arm, one vector off the unit sphere.
        let path = dir.path().join("legacy.vectors");
        let mut bytes = std::fs::read(&path).expect("test: read back");
        bytes[flag_at] = 0;
        if off_sphere {
            let width = std::mem::size_of::<f32>();
            for j in 0..DIM {
                let at = payload + j * width;
                let value =
                    f32::from_le_bytes(bytes[at..at + width].try_into().expect("test: f32"));
                bytes[at..at + width].copy_from_slice(&(value * 3.0).to_le_bytes());
            }
        }
        std::fs::write(&path, &bytes).expect("test: write back");

        let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, DIM);
        let loaded = H::file_load(dir.path(), "legacy", engine).expect("test: load");

        assert_eq!(
            std::fs::read(&path).expect("test: read after"),
            bytes,
            "off_sphere={off_sphere}: opening wrote the durable file"
        );
        let (norm, adopted) = loaded
            .vectors
            .read()
            .as_ref()
            .map(|vectors| {
                (
                    crate::simd_native::norm_native(vectors.get(0).expect("test: first vector")),
                    vectors.backing_path() == Some(path.as_path()),
                )
            })
            .expect("test: vectors loaded");
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "off_sphere={off_sphere}: the loaded vector must be unit-norm, got {norm}"
        );
        assert_eq!(
            adopted, expect_adopted,
            "off_sphere={off_sphere}: whether the arena is still the file"
        );
    }
}

/// Writes a pre-flag cosine dump holding one vector off the unit sphere and
/// returns its `.vectors` path and bytes: the payload a load must copy out of
/// the adopted file before normalizing it.
fn legacy_off_sphere_dump(dir: &std::path::Path) -> (std::path::PathBuf, Vec<u8>) {
    use crate::perf_optimizations::ContiguousVectors;
    const DIM: usize = 4;
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, DIM);
    let hnsw = NativeHnsw::new(engine, 16, 100, 100);
    for i in 0..ContiguousVectors::MIN_ARENA_CAPACITY + 2 {
        hnsw.insert(&[i as f32 + 1.0, 0.5, 0.25, 2.0])
            .expect("test: insert");
    }
    hnsw.file_dump(dir, "legacy").expect("test: dump");
    drop(hnsw);
    let path = dir.join("legacy.vectors");
    let mut bytes = std::fs::read(&path).expect("test: read back");
    bytes[usize::try_from(super::VECTORS_HEADER_BYTES).expect("test: offset fits usize")] = 0;
    let payload = usize::try_from(super::VECTORS_V2_DATA_OFFSET).expect("test: offset fits usize");
    let width = std::mem::size_of::<f32>();
    for j in 0..DIM {
        let at = payload + j * width;
        let value = f32::from_le_bytes(bytes[at..at + width].try_into().expect("test: f32"));
        bytes[at..at + width].copy_from_slice(&(value * 3.0).to_le_bytes());
    }
    std::fs::write(&path, &bytes).expect("test: write back");
    (path, bytes)
}

/// A store whose mode keeps a disposable arena (SQ8, `RaBitQ`) gets the copy
/// there rather than on the heap, so its f32 payload stays evictable (#2246).
#[test]
fn a_legacy_payload_is_copied_into_the_arena_its_mode_keeps() {
    let dir = tempdir().expect("test: tempdir");
    let (path, bytes) = legacy_off_sphere_dump(dir.path());
    let engine = CachedSimdDistance::new_prenormalized(DistanceMetric::Cosine, 4);
    let loaded = H::file_load_with_arena(dir.path(), "legacy", engine, Some(dir.path()))
        .expect("test: load");
    assert_eq!(
        std::fs::read(&path).expect("test: read after"),
        bytes,
        "opening wrote the durable file"
    );
    let backing = loaded
        .vectors
        .read()
        .as_ref()
        .and_then(|vectors| vectors.backing_path().map(std::path::Path::to_path_buf))
        .expect("test: the copy must be file-backed, not on the heap");
    assert_ne!(backing, path, "the copy must not be the durable file");
    assert_eq!(
        backing.extension().and_then(|ext| ext.to_str()),
        Some("arena"),
        "the copy must live in the mode's disposable arena: {backing:?}"
    );
    // The graph owns that arena: it lives exactly as long as the graph.
    assert!(
        backing.exists(),
        "the arena must exist while the graph lives"
    );
    drop(loaded);
    assert!(!backing.exists(), "the arena must go with the graph");
}
