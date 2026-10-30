use super::*;

/// Compile-time assertion that `io_holder` field is declared AFTER `inner`.
#[test]
fn test_field_order_io_holder_after_inner() {
    use std::mem::offset_of;

    let inner_offset = offset_of!(HnswIndex, inner);
    let io_holder_offset = offset_of!(HnswIndex, io_holder);

    assert!(
        inner_offset < io_holder_offset,
        "CRITICAL: io_holder must be declared AFTER inner for correct drop order"
    );
}

/// A huge `max_elements` is a capacity hint, not a reservation. The base
/// layer and the id mappings used to be reserved at it up front: on a SQ8 or
/// `RaBitQ` index with an arena directory (whose own arena is capped), a
/// caller's `usize::MAX` reached them and aborted the process. The index is
/// now built with a bounded reservation and still takes inserts.
#[cfg(feature = "persistence")]
#[test]
fn test_a_huge_max_elements_builds_a_working_quantized_index() {
    for storage_mode in [crate::StorageMode::SQ8, crate::StorageMode::RaBitQ] {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let params = crate::index::hnsw::HnswParams {
            max_elements: usize::MAX,
            storage_mode,
            ..crate::index::hnsw::HnswParams::auto(8)
        };
        let index = HnswIndex::with_params_in_dir(
            8,
            DistanceMetric::Euclidean,
            params,
            true,
            Some(dir.path().to_path_buf()),
        )
        .expect("test: a huge max_elements builds the index");

        crate::index::VectorIndex::insert(&index, 1, &[0.5; 8]);
        assert_eq!(
            crate::index::VectorIndex::len(&index),
            1,
            "{storage_mode:?}"
        );
    }
}
