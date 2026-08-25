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

/// Reordering a file-backed arena keeps it file-backed.
///
/// The regression this pins: `reorder` used to gather into a fresh heap
/// buffer and swap the backing to `Heap`, which silently cost the arena the
/// one property it exists for — its pages stopped being evictable — and made
/// `flush_backing` a no-op that still returned `Ok`. Nothing in the vectors
/// read back would have shown it, which is why the backing is asserted
/// directly.
#[test]
fn reorder_keeps_a_mapped_arena_mapped() {
    let dir = tempdir().expect("tempdir");
    let dimension = 4;
    let mut storage =
        ContiguousVectors::new_file_backed(&dir.path().join("o.arena"), dimension, 16)
            .expect("file arena");
    let written = fill(&mut storage, 4, dimension);

    storage.reorder(&[3, 2, 1, 0]).expect("reorder");

    let expected: Vec<Vec<f32>> = written.iter().rev().cloned().collect();
    assert_contents(&storage, &expected);
    assert!(
        format!("{storage:?}").contains("FileMapped"),
        "arena must still be file-mapped after a reorder, got: {storage:?}"
    );
    // The capacity the file was sized for survives too: an in-place
    // permutation has no reason to shrink it, and shrinking would make the
    // next push re-map.
    assert_eq!(storage.capacity(), 16, "reorder must not resize the arena");
    drop(storage);
}

/// The reordered bytes are the ones on disk.
///
/// `flush_backing` reaching the file is the observable end of "the mapping
/// survived": with the pre-#2112 heap demotion this file still held the
/// insertion-order bytes, and a reopen would have served them.
#[test]
fn reorder_writes_the_new_order_through_to_the_file() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("persisted.arena");
    let dimension = 4;
    let mut storage = ContiguousVectors::new_file_backed(&path, dimension, 16).expect("file arena");
    let written = fill(&mut storage, 4, dimension);

    storage.reorder(&[3, 2, 1, 0]).expect("reorder");
    storage.flush_backing().expect("flush");

    // Read the raw file rather than reopening: the arena still holds its
    // exclusive lock, and reading the bytes underneath a live mapping is what
    // proves `flush_backing` reached the file — a reopen would also pass on
    // the writeback that `munmap` does anyway.
    let expected: Vec<f32> = written.iter().rev().flatten().copied().collect();
    assert_eq!(on_disk_vectors(&path, expected.len()), expected);
}

/// Reads `len` f32s from an arena file's data section.
///
/// Native byte order, because that is what the mapping wrote — the v1 layout
/// is deliberately host-native (#2112); a portable one is tracked separately.
fn on_disk_vectors(path: &std::path::Path, len: usize) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("read arena file");
    let data = &bytes[DATA_OFFSET..];
    data.chunks_exact(std::mem::size_of::<f32>())
        .take(len)
        .map(|c| {
            let mut word = [0_u8; 4];
            word.copy_from_slice(c);
            f32::from_ne_bytes(word)
        })
        .collect()
}

/// A `new_order` that is not a bijection is refused, not applied.
///
/// The copying implementation accepted one: a repeated index duplicated a
/// vector and dropped whichever index was missing, with no error. The
/// in-place algorithm walks cycles, so the same input would not terminate —
/// validating up front is what turns a hang into a message.
#[test]
fn reorder_refuses_a_permutation_that_repeats_an_index() {
    let dir = tempdir().expect("tempdir");
    let dimension = 4;
    let mut storage =
        ContiguousVectors::new_file_backed(&dir.path().join("dup.arena"), dimension, 16)
            .expect("file arena");
    let written = fill(&mut storage, 4, dimension);

    let err = storage
        .reorder(&[0, 1, 1, 3])
        .expect_err("a repeated index is not a permutation");
    assert!(
        err.to_string().contains("appears twice"),
        "error should name the violation, got: {err}"
    );
    assert_contents(&storage, &written);
}

/// A mapped arena and a heap arena come out of the same permutation identical.
///
/// The arena's claim is indistinguishability; a reorder is the operation
/// where the two backings most recently diverged, so it is checked rather
/// than assumed.
#[test]
fn reorder_agrees_between_the_two_backings() {
    let dir = tempdir().expect("tempdir");
    let dimension = 3;
    let count = 9;
    // A single 9-cycle plus a fixed point exercises both branches of the
    // outer loop: `[1, 2, 3, 4, 5, 6, 7, 0, 8]` maps slot 8 to itself.
    let order = [1, 2, 3, 4, 5, 6, 7, 0, 8];

    let mut mapped =
        ContiguousVectors::new_file_backed(&dir.path().join("cmp.arena"), dimension, 16)
            .expect("file arena");
    let written = fill(&mut mapped, count, dimension);
    let mut heap = ContiguousVectors::new(dimension, 16).expect("heap arena");
    for v in &written {
        heap.push(v).expect("push");
    }

    mapped.reorder(&order).expect("reorder mapped");
    heap.reorder(&order).expect("reorder heap");

    let expected: Vec<Vec<f32>> = order.iter().map(|&old| written[old].clone()).collect();
    assert_contents(&mapped, &expected);
    assert_contents(&heap, &expected);
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

/// Eviction must not change a single vector.
///
/// The property callers depend on: dropping the resident pages is invisible
/// to anyone reading the arena. Everything above `ContiguousVectors` treats a
/// mapped arena as memory, and a re-rank that scored against re-faulted pages
/// holding anything other than the original vectors would return wrong
/// neighbours with no error anywhere.
///
/// Note what this does *not* pin. It passes with `evict`'s flush removed,
/// which was checked: on Linux the mapping's pages are the file's page-cache
/// pages, so `MADV_DONTNEED` cannot lose a write. The flush is there to make
/// a subsequent page-cache drop deterministic, not to keep this test green.
///
/// Sized to span many pages — a single-page arena would pass on luck.
#[test]
fn evicting_preserves_every_vector() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("evict.arena");
    let dimension = 128;
    let count = 512; // 256 KiB — 64 pages, well past a single-page fluke.

    let mut arena = ContiguousVectors::new_file_backed(&path, dimension, count).expect("create");
    let written = fill(&mut arena, count, dimension);

    arena.evict_backing().expect("eviction succeeds");

    for (i, expected) in written.iter().enumerate() {
        assert_eq!(
            arena.get(i).expect("slot present"),
            expected.as_slice(),
            "vector {i} changed across an evict; the flush before MADV_DONTNEED is missing \
             or ineffective, and a re-rank would score against wrong vectors"
        );
    }
}

/// Eviction is a no-op on a heap arena, not an error.
///
/// The measurement path calls this on whatever arena it is handed. A heap
/// arena has no file to drop pages to, and that asymmetry is the feature —
/// it must not surface as a failure.
#[test]
fn evicting_a_heap_arena_is_a_no_op() {
    let dimension = 8;
    let mut arena = ContiguousVectors::new(dimension, 16).expect("heap arena");
    let written = fill(&mut arena, 16, dimension);

    arena.evict_backing().expect("a heap arena reports success");

    for (i, expected) in written.iter().enumerate() {
        assert_eq!(arena.get(i).expect("slot present"), expected.as_slice());
    }
}
