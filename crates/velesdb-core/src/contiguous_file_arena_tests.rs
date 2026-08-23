//! Contracts for the file-backed vector arena (#2112 Phase B).
//!
//! The arena's whole claim is that it is *indistinguishable* from the
//! anonymous one apart from where its bytes live. These tests are written as
//! equivalence checks against a heap-backed arena wherever a behaviour is
//! shared, and only assert file-specific facts (the backing survives growth,
//! the bytes reach disk) where there is nothing to compare against.

use crate::contiguous_file_arena::DATA_OFFSET;
use crate::perf_optimizations::ContiguousVectors;
use tempfile::tempdir;

/// Deterministic vector so a failure names the slot, not a random seed.
///
/// Values are built from `u16` and widened losslessly, so the assertions
/// compare exact floats rather than whatever a `usize as f32` cast rounded to.
fn vector(dimension: usize, seed: usize) -> Vec<f32> {
    (0..dimension)
        .map(|d| {
            let raw = u16::try_from((seed * dimension + d) % 65_536).expect("fits u16");
            f32::from(raw)
        })
        .collect()
}

/// Fills `storage` with `n` vectors and returns what was written.
fn fill(storage: &mut ContiguousVectors, n: usize, dimension: usize) -> Vec<Vec<f32>> {
    let written: Vec<Vec<f32>> = (0..n).map(|i| vector(dimension, i)).collect();
    for v in &written {
        storage.push(v).expect("push");
    }
    written
}

/// Every slot reads back exactly what was written, in order.
fn assert_contents(storage: &ContiguousVectors, expected: &[Vec<f32>]) {
    assert_eq!(storage.len(), expected.len(), "vector count");
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            storage.get(i).expect("slot present"),
            want.as_slice(),
            "slot {i} content"
        );
    }
}

/// A file-backed arena stores and returns vectors like the heap one.
#[test]
fn file_backed_matches_heap_backed_contents() {
    let dir = tempdir().expect("tempdir");
    let dimension = 32;

    let mut heap = ContiguousVectors::new(dimension, 16).expect("heap arena");
    let mut mapped = ContiguousVectors::new_file_backed(&dir.path().join("a.arena"), dimension, 16)
        .expect("file arena");

    let written = fill(&mut mapped, 40, dimension);
    for v in &written {
        heap.push(v).expect("push");
    }

    assert_contents(&mapped, &written);
    assert_eq!(
        mapped.as_flat_slice(),
        heap.as_flat_slice(),
        "the two backings must produce byte-identical arenas"
    );
}

/// Growth past the initial capacity must re-map without losing a byte.
///
/// This is the path where the two backings genuinely differ: the heap arena
/// copies into a bigger block, the file arena extends the file and re-maps.
/// The pointer moves in both cases, so the contents are the contract.
#[test]
fn growth_preserves_every_vector_across_remap() {
    let dir = tempdir().expect("tempdir");
    let dimension = 8;
    let mut storage =
        ContiguousVectors::new_file_backed(&dir.path().join("g.arena"), dimension, 16)
            .expect("file arena");
    assert_eq!(storage.capacity(), 16, "starts at the requested capacity");

    // 500 vectors forces several doublings past the initial 16.
    let written = fill(&mut storage, 500, dimension);
    assert!(
        storage.capacity() >= 500,
        "capacity must have grown, got {}",
        storage.capacity()
    );
    assert_contents(&storage, &written);
}

/// A grown arena's freshly appended range reads as zeros.
///
/// `insert_at` may leave gaps, and relies on the anonymous backing's
/// `alloc_zeroed` guarantee. An extended file must offer the same.
#[test]
fn grown_region_is_zero_filled() {
    let dir = tempdir().expect("tempdir");
    let dimension = 4;
    let mut storage =
        ContiguousVectors::new_file_backed(&dir.path().join("z.arena"), dimension, 16)
            .expect("file arena");

    // Write far past the initial capacity, leaving every earlier slot untouched.
    storage
        .insert_at(200, &vector(dimension, 200))
        .expect("insert_at");

    for gap in [0_usize, 1, 99, 199] {
        assert_eq!(
            storage.get(gap).expect("gap slot present"),
            vec![0.0_f32; dimension].as_slice(),
            "slot {gap} must be zero-filled, not uninitialised"
        );
    }
}

/// Reopening the file yields the same vectors, without deserialization.
#[test]
fn reopening_the_file_recovers_the_arena() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("r.arena");
    let dimension = 16;
    let count = 50;

    let written = {
        let mut storage =
            ContiguousVectors::new_file_backed(&path, dimension, 64).expect("file arena");
        let written = fill(&mut storage, count, dimension);
        storage.flush_backing().expect("flush");
        written
    };

    let reopened =
        ContiguousVectors::open_file_backed(&path, dimension, 64, count).expect("reopen");
    assert_contents(&reopened, &written);
}

/// The data region starts a page in, so the file carries its reserved header.
#[test]
fn file_reserves_a_page_before_the_data_region() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("h.arena");
    let dimension = 8;
    let mut storage = ContiguousVectors::new_file_backed(&path, dimension, 16).expect("file arena");
    fill(&mut storage, 16, dimension);
    storage.flush_backing().expect("flush");

    let len = std::fs::metadata(&path).expect("metadata").len();
    let data_bytes = (16 * dimension * std::mem::size_of::<f32>()) as u64;
    assert_eq!(
        len,
        DATA_OFFSET as u64 + data_bytes,
        "file is a reserved page followed by exactly the vector bytes"
    );
}

/// `open_file_backed` refuses a count that cannot fit the declared capacity.
#[test]
fn open_refuses_a_count_beyond_capacity() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("bad.arena");
    let dimension = 4;
    drop(ContiguousVectors::new_file_backed(&path, dimension, 16).expect("file arena"));

    let err = ContiguousVectors::open_file_backed(&path, dimension, 16, 17)
        .expect_err("count > capacity must be refused");
    assert!(
        err.to_string().contains("exceeds capacity"),
        "error should name the violated bound, got: {err}"
    );
}

/// Reordering a file-backed arena moves it onto the heap and frees the map.
///
/// The dangerous shape this pins: `reorder` allocates a fresh heap buffer and
/// deallocates the old one. If the backing were not switched first, that
/// `dealloc` would be handed a mapped pointer.
#[test]
fn reorder_moves_a_mapped_arena_onto_the_heap() {
    let dir = tempdir().expect("tempdir");
    let dimension = 4;
    let mut storage =
        ContiguousVectors::new_file_backed(&dir.path().join("o.arena"), dimension, 16)
            .expect("file arena");
    let written = fill(&mut storage, 4, dimension);

    storage.reorder(&[3, 2, 1, 0]).expect("reorder");

    let expected: Vec<Vec<f32>> = written.iter().rev().cloned().collect();
    assert_contents(&storage, &expected);
    // Dropping here exercises the heap path on a formerly mapped arena; a
    // wrong backing would fault or corrupt the allocator under Miri/ASan.
    drop(storage);
}

/// The mapped data pointer is f32-aligned — a soundness invariant, not a
/// preference.
///
/// `ContiguousVectors` hands out `&[f32]` via `slice::from_raw_parts`, whose
/// contract requires alignment. A misaligned arena would be undefined
/// behaviour at the first `get`, not merely slow, so this is pinned rather
/// than left to the arithmetic in `DATA_OFFSET`.
#[test]
fn mapped_data_pointer_is_aligned_for_f32_slices() {
    let dir = tempdir().expect("tempdir");
    let dimension = 3; // odd, so a stride bug cannot hide behind a power of two
    let mut storage =
        ContiguousVectors::new_file_backed(&dir.path().join("al.arena"), dimension, 16)
            .expect("file arena");
    fill(&mut storage, 40, dimension); // forces a re-map partway through

    let addr = storage.as_flat_slice().as_ptr() as usize;
    assert_eq!(
        addr % std::mem::align_of::<f32>(),
        0,
        "data pointer {addr:#x} must satisfy from_raw_parts' alignment contract"
    );
    assert_eq!(
        addr % DATA_OFFSET,
        0,
        "data region should start on the page boundary DATA_OFFSET promises"
    );
}

/// An arena file is native-endian, so a round-trip through it is only
/// meaningful on one machine.
///
/// Pins the constraint the module documents: the bytes are the arena, not a
/// converted interchange form. On a little-endian target the mapped bytes
/// therefore match `f32::to_le_bytes`; the assertion is written so it states
/// what it depends on rather than silently assuming it.
#[test]
#[cfg(target_endian = "little")]
fn arena_bytes_are_the_raw_native_representation() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("e.arena");
    let dimension = 2;
    let mut storage = ContiguousVectors::new_file_backed(&path, dimension, 16).expect("file arena");
    storage.push(&[1.5_f32, -2.25]).expect("push");
    storage.flush_backing().expect("flush");

    let raw = std::fs::read(&path).expect("read arena file");
    let data = &raw[DATA_OFFSET..DATA_OFFSET + 8];
    let mut expected = Vec::new();
    expected.extend_from_slice(&1.5_f32.to_le_bytes());
    expected.extend_from_slice(&(-2.25_f32).to_le_bytes());
    assert_eq!(
        data, expected,
        "the data region is the f32 values themselves, unconverted"
    );
}

/// A second arena over the same file is refused.
///
/// This is the invariant the `unsafe impl Send`/`Sync` on `ContiguousVectors`
/// depend on. A heap arena is unique because the allocator says so; a path is
/// not, and two mappings of one file are two `&mut [f32]` aliases of the same
/// bytes. Enforcing it beats documenting it, so the second open fails.
#[test]
fn a_second_arena_over_the_same_file_is_refused() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("excl.arena");
    let first = ContiguousVectors::new_file_backed(&path, 8, 16).expect("first arena");

    let err = ContiguousVectors::new_file_backed(&path, 8, 16)
        .expect_err("a second mapping of the same file must be refused");
    // A held arena is a locked resource, not an I/O mishap: the caller can
    // tell "someone else has this" from "the disk is full".
    assert!(
        matches!(err, crate::error::Error::DatabaseLocked(ref p) if p.contains("excl.arena")),
        "a refused lock should surface as DatabaseLocked naming the path, got: {err:?}"
    );

    // Releasing the first frees the file for a later holder — the lock is
    // scoped to the arena's life, not to the process.
    drop(first);
    drop(ContiguousVectors::new_file_backed(&path, 8, 16).expect("reopen after release"));
}

/// A refused second arena must not have already destroyed the first's bytes.
///
/// The hazard is ordering, not aliasing: `create` used to pass
/// `truncate(true)` to `OpenOptions`, which empties the file at open time —
/// *before* the exclusive lock is taken. A call destined to be refused had
/// therefore already discarded the bytes the winning arena was mapping, and
/// that arena's mapping then addressed pages past a shrunken end of file,
/// where a read raises `SIGBUS`. The lock existed precisely to stop a second
/// holder touching these bytes, and the truncate flag walked around it.
///
/// This test failed with `signal: 7, SIGBUS` before the fix, which is why it
/// asserts on the *first* arena rather than on the refusal alone: refusing
/// correctly while having already destroyed the file is the bug.
#[test]
fn a_refused_second_arena_leaves_the_first_intact() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("trunc.arena");
    let dimension = 4;
    let mut first = ContiguousVectors::new_file_backed(&path, dimension, 16).expect("first arena");
    let written = fill(&mut first, 8, dimension);

    let err = ContiguousVectors::new_file_backed(&path, dimension, 16)
        .expect_err("a second mapping of the same file must be refused");
    assert!(
        matches!(err, crate::error::Error::DatabaseLocked(_)),
        "expected a locked-resource error, got: {err:?}"
    );

    // Reading through the surviving mapping is the assertion: if the refused
    // call had resized the file, this faults instead of comparing.
    assert_contents(&first, &written);
}

/// Creating over a stale file still yields a zeroed arena.
///
/// Moving the discard out of `OpenOptions` and under the lock must not lose
/// the guarantee it provided: `create` starts from zeros, never from whatever
/// a previous, larger arena left behind.
#[test]
fn creating_over_an_existing_file_discards_its_contents() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("stale.arena");
    let dimension = 4;

    {
        let mut old = ContiguousVectors::new_file_backed(&path, dimension, 64).expect("first");
        fill(&mut old, 64, dimension);
        old.flush_backing().expect("flush");
    }

    let mut fresh = ContiguousVectors::new_file_backed(&path, dimension, 16).expect("recreate");
    fresh
        .insert_at(8, &vector(dimension, 8))
        .expect("insert_at");

    for gap in [0_usize, 7] {
        assert_eq!(
            fresh.get(gap).expect("gap slot present"),
            vec![0.0_f32; dimension].as_slice(),
            "slot {gap} must be zeroed, not carried over from the old arena"
        );
    }
}
