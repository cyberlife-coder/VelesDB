use super::*;
use crate::store_insert::insert_with_payload;
use crate::store_new::create_store;

#[test]
fn test_metric_roundtrip() {
    let metrics = [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ];
    for metric in metrics {
        let byte = metric_to_byte(metric);
        let result = byte_to_metric(byte).unwrap();
        assert_eq!(metric, result);
    }
}

const ALL_MODES: [StorageMode; 5] = [
    StorageMode::Full,
    StorageMode::SQ8,
    StorageMode::Binary,
    StorageMode::ProductQuantization,
    StorageMode::RaBitQ,
];

#[test]
fn test_v3_roundtrip_preserves_mode_vectors_and_payloads_in_every_mode() {
    for mode in ALL_MODES {
        let mut store = create_store(3, DistanceMetric::Euclidean, mode);
        insert_with_payload(
            &mut store,
            1,
            &[0.25, -1.5, 3.0],
            Some(serde_json::json!({"k": "v"})),
        );
        insert_with_payload(&mut store, 2, &[-0.75, 0.5, 2.0], None);

        let bytes = export_to_bytes(&store);
        assert_eq!(bytes[4], 3, "{mode:?}: export writes format v3");
        let restored = import_from_bytes(&bytes).unwrap();

        assert_eq!(restored.storage_mode, mode);
        assert_eq!(restored.ids, vec![1, 2]);
        assert_eq!(
            restored.data,
            vec![0.25, -1.5, 3.0, -0.75, 0.5, 2.0],
            "{mode:?}: the f32 vectors survive the round trip exactly"
        );
        assert_eq!(restored.payloads[0], Some(serde_json::json!({"k": "v"})));
        assert_eq!(restored.payloads[1], None);
    }
}

#[test]
fn test_v3_size_of_one_3d_vector_without_payload() {
    // Header 19 + id 8 + data blob (8 + 3 * 4) + empty payload blob 8 = 55
    // (`WASM_PERSISTENCE.md`; the v2 export of the same store was 87 bytes).
    let mut store = create_store(3, DistanceMetric::Cosine, StorageMode::SQ8);
    insert_with_payload(&mut store, 1, &[1.0, 0.0, 0.0], None);
    assert_eq!(export_to_bytes(&store).len(), 55);
}

// ---------------------------------------------------------------------------
// Legacy v2 blobs.
//
// Byte-for-byte exports of the v2 build (before every mode kept f32): six
// 8-dimension vectors, ids 10..=15, Euclidean, id 12 carrying the payload
// `{"tag": "two"}`, in `full`, `sq8` and `binary` mode. The v2 `pq` and
// `rabitq` exports are the `sq8` bytes with the mode byte (offset 10) set to
// 3 and 4: v2 stored all three through the same SQ8 encoder. Each fixture
// comes with the ranking that build returned for `LEGACY_QUERY`; loading the
// blob today must reproduce it, since the reconstruction the old build
// searched over becomes the f32 vectors the store now keeps.
// ---------------------------------------------------------------------------

const V2_FULL_HEX: &str = "\
    56454c530208000000010006000000000000000a000000000000000b000000000000000c00000000\
    0000000d000000000000000e000000000000000f00000000000000c0000000000000000000a0bf00\
    0000bf0000803e0000803f000080bf000080be0000003f0000a03f9a99193fcdccac3f666626bfcd\
    cccc3d9a99593f333393bfcdccccbe3333b33e9a9999be6666e63e9a99993fcdcc4cbfcccc4cbd33\
    33333f9a99b93fcdcc0cbf6666c63f6666e6be9a99993e6666863f333373bfcccc4cbecdcc0c3f66\
    66a63f6666263f3333b33f9a9919bf9a99193e6666663f3333d33f3333b3becdcccc3e000080be00\
    00003f0000a03f000040bf000000000000403f0000c03f000000bf00000000000000000000000000\
    00000000000000000000000000000000000000000000000000000000000000000000000d00000000\
    0000007b22746167223a2274776f227d000000000000000000000000000000000000000000000000";

const V2_SQ8_HEX: &str = "\
    56454c530208000000010106000000000000000a000000000000000b000000000000000c00000000\
    0000000d000000000000000e000000000000000f0000000000000000000000000000003000000000\
    000000004d99e61a66b3ffb3ff3380cc004d99398ee30055aaff1cff3380cc004d99e68ee30055aa\
    ff1c71398ee30055aaff1c000000000000000018000000000000000000a0bf333393bfcdcc4cbf33\
    3373bf9a9919bf000040bf18000000000000000000cc420000cc42abaae2420000cc42abaae242ab\
    aae242000000000000000000000000000000000d000000000000007b22746167223a2274776f227d\
    000000000000000000000000000000000000000000000000";

const V2_BINARY_HEX: &str = "\
    56454c530208000000010206000000000000000a000000000000000b000000000000000c00000000\
    0000000d000000000000000e000000000000000f0000000000000000000000000000000000000000\
    0000000600000000000000cc9b66cdbb760000000000000000000000000000000000000000000000\
    0000000000000000000d000000000000007b22746167223a2274776f227d00000000000000000000\
    0000000000000000000000000000";

const LEGACY_QUERY: [f32; 8] = [0.4, -0.3, 1.1, 0.2, -0.9, 0.5, 0.05, -0.6];

/// Ranking the v2 build returned for `LEGACY_QUERY` on the SQ8 fixture (and
/// on its `pq`/`rabitq` twins).
const V2_SQ8_RANKING: [(u64, f32); 6] = [
    (12, 2.188_178),
    (15, 2.230_182),
    (13, 2.657_817_4),
    (10, 2.886_504_7),
    (14, 3.402_275),
    (11, 3.566_741),
];

/// Score tolerance against the v2 build: the SQ8 reconstruction now goes
/// through core's `QuantizedVector::to_f32` (`q * range / 255 + min`) where v2
/// used `q / scale + min`, which can move a decoded value by a few ULP.
const LEGACY_TOLERANCE: f32 = 1.0e-4;

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// Loads a v2 fixture and asserts it ranks `LEGACY_QUERY` as the v2 build did:
/// the same score at every rank, and every returned id carrying its v2 score
/// (so only ids the v2 build tied may swap places).
fn assert_legacy_ranking(bytes: &[u8], mode: StorageMode, expected: &[(u64, f32)]) {
    let store = import_from_bytes(bytes).unwrap();
    assert_eq!(store.storage_mode, mode);
    assert_eq!(store.data.len(), 6 * 8, "{mode:?}: one f32 vector per id");
    assert_eq!(
        store.payload_for_id(12),
        Some(&serde_json::json!({"tag": "two"}))
    );

    let ranking = store.search_scored(&LEGACY_QUERY, expected.len());
    assert_eq!(ranking.len(), expected.len());
    for (rank, ((id, score), (_, want))) in ranking.iter().zip(expected).enumerate() {
        assert!(
            (score - want).abs() <= LEGACY_TOLERANCE,
            "{mode:?} rank {rank}: score {score}, v2 build had {want}"
        );
        let v2_score = expected
            .iter()
            .find(|(v2_id, _)| v2_id == id)
            .map(|(_, s)| *s);
        assert!(
            v2_score.is_some_and(|s| (s - score).abs() <= LEGACY_TOLERANCE),
            "{mode:?} rank {rank}: id {id} scored {score}, v2 build had {v2_score:?}"
        );
    }
}

#[test]
fn test_legacy_v2_full_store_loads_unchanged() {
    assert_legacy_ranking(
        &hex_bytes(V2_FULL_HEX),
        StorageMode::Full,
        &[
            (12, 2.188_035_7),
            (15, 2.229_910_4),
            (13, 2.657_066),
            (10, 2.884_874_3),
            (14, 3.401_102_8),
            (11, 3.567_211_9),
        ],
    );
}

#[test]
fn test_legacy_v2_sq8_pq_rabitq_stores_load_with_their_v2_ranking() {
    for (mode, byte) in [
        (StorageMode::SQ8, 1u8),
        (StorageMode::ProductQuantization, 3),
        (StorageMode::RaBitQ, 4),
    ] {
        let mut bytes = hex_bytes(V2_SQ8_HEX);
        bytes[10] = byte;
        assert_legacy_ranking(&bytes, mode, &V2_SQ8_RANKING);
    }
}

#[test]
fn test_legacy_v2_binary_store_loads_with_its_v2_ranking() {
    // Ids 11 and 14 tie in the v2 build; either order is that build's answer.
    assert_legacy_ranking(
        &hex_bytes(V2_BINARY_HEX),
        StorageMode::Binary,
        &[
            (12, 2.054_872_3),
            (10, 2.328_626_2),
            (13, 2.371_181_2),
            (15, 2.65),
            (11, 3.212_864_9),
            (14, 3.212_864_9),
        ],
    );
}

#[test]
fn test_legacy_v2_binary_decodes_sign_bits_to_zero_and_one() {
    let store = import_from_bytes(&hex_bytes(V2_BINARY_HEX)).unwrap();
    // First vector: [-1.25, -0.5, 0.25, 1.0, -1.0, -0.25, 0.5, 1.25].
    assert_eq!(&store.data[..8], &[0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
}

#[test]
fn test_legacy_sq8_sentinels_decode_like_core() {
    // v2's per-vector scale sentinels: `0.0` (degenerate range, decodes to
    // `min`) and `NaN` (non-finite range, decodes to `NaN`).
    let buffers = legacy::LegacyBuffers {
        data: Vec::new(),
        data_sq8: vec![128, 128, 0, 0],
        data_binary: Vec::new(),
        sq8_mins: vec![0.5, 1.0],
        sq8_scales: vec![0.0, f32::NAN],
    };
    let data = legacy::reconstruct_vectors(StorageMode::SQ8, 2, 2, buffers).unwrap();
    assert_eq!(&data[..2], &[0.5, 0.5]);
    assert!(data[2..].iter().all(|v| v.is_nan()));
}

#[test]
fn test_legacy_buffers_of_the_wrong_length_are_rejected() {
    // A v2 SQ8 store written through the old `insert_batch`, which appended
    // to the f32 buffer whatever the mode: codes for one row, two ids.
    let buffers = legacy::LegacyBuffers {
        data: vec![0.0; 2],
        data_sq8: vec![1, 2],
        data_binary: Vec::new(),
        sq8_mins: vec![0.0],
        sq8_scales: vec![1.0],
    };
    let err = legacy::reconstruct_vectors(StorageMode::SQ8, 2, 2, buffers).unwrap_err();
    assert!(err.contains("data_sq8"), "{err}");
}

#[test]
fn test_vector_count_must_match_ids_and_dimension() {
    assert!(check_vector_count(6, 2, 3).is_ok());
    assert!(check_vector_count(5, 2, 3).is_err());
    assert!(check_vector_count(0, usize::MAX, 2).is_err());
}
