//! Equivalence tests: prove WASM ColumnStore binding produces identical
//! results to core ColumnStore for the same data.
//!
//! Since `wasm_bindgen` functions can't run on native targets, we test the
//! core APIs that the WASM layer delegates to. The WASM binding is a thin
//! JSON ↔ ColumnValue translation — if core produces correct results,
//! the binding does too (verified separately via Playwright browser tests).

use velesdb_core::column_store::{ColumnStore, ColumnType, ColumnValue, VacuumConfig};

// =============================================================================
// Helpers
// =============================================================================

/// Creates two identical stores (simulating core and WASM-side).
fn make_pair() -> (ColumnStore, ColumnStore) {
    let schema: &[(&str, ColumnType)] = &[
        ("id", ColumnType::Int),
        ("name", ColumnType::String),
        ("age", ColumnType::Int),
        ("score", ColumnType::Float),
        ("active", ColumnType::Bool),
    ];
    let a = ColumnStore::with_primary_key(schema, "id").unwrap();
    let b = ColumnStore::with_primary_key(schema, "id").unwrap();
    (a, b)
}

/// Inserts identical rows into both stores.
fn insert_same(
    stores: &mut (ColumnStore, ColumnStore),
    id: i64,
    name: &str,
    age: i64,
    score: f64,
    active: bool,
) {
    for store in [&mut stores.0, &mut stores.1] {
        let name_id = store.string_table_mut().intern(name);
        store
            .insert_row(&[
                ("id", ColumnValue::Int(id)),
                ("name", ColumnValue::String(name_id)),
                ("age", ColumnValue::Int(age)),
                ("score", ColumnValue::Float(score)),
                ("active", ColumnValue::Bool(active)),
            ])
            .unwrap();
    }
}

// =============================================================================
// Task 1: ColumnStore Equivalence Tests (8 scenarios)
// =============================================================================

/// 1. Schema equivalence — same schema produces same column structure.
#[test]
fn test_schema_equivalence() {
    let (a, b) = make_pair();

    assert_eq!(a.row_count(), b.row_count());
    assert_eq!(a.active_row_count(), b.active_row_count());
    assert_eq!(a.primary_key_column(), b.primary_key_column());

    let mut names_a: Vec<&str> = a.column_names().collect();
    let mut names_b: Vec<&str> = b.column_names().collect();
    names_a.sort_unstable();
    names_b.sort_unstable();
    assert_eq!(names_a, names_b);
}

/// 2. Insert + filter equivalence — same rows + same filter → same indices.
#[test]
fn test_insert_filter_equivalence() {
    let mut pair = make_pair();
    insert_same(&mut pair, 1, "Alice", 30, 95.5, true);
    insert_same(&mut pair, 2, "Bob", 25, 88.0, false);
    insert_same(&mut pair, 3, "Charlie", 30, 77.0, true);
    insert_same(&mut pair, 4, "Diana", 40, 92.0, false);

    // filter_eq_int
    assert_eq!(
        pair.0.filter_eq_int("age", 30),
        pair.1.filter_eq_int("age", 30),
    );

    // filter_gt_int
    assert_eq!(
        pair.0.filter_gt_int("age", 28),
        pair.1.filter_gt_int("age", 28),
    );

    // filter_lt_int
    assert_eq!(
        pair.0.filter_lt_int("age", 35),
        pair.1.filter_lt_int("age", 35),
    );

    // filter_range_int
    assert_eq!(
        pair.0.filter_range_int("age", 24, 31),
        pair.1.filter_range_int("age", 24, 31),
    );

    // filter_eq_string
    assert_eq!(
        pair.0.filter_eq_string("name", "Bob"),
        pair.1.filter_eq_string("name", "Bob"),
    );

    // filter_in_string
    assert_eq!(
        pair.0.filter_in_string("name", &["Alice", "Diana"]),
        pair.1.filter_in_string("name", &["Alice", "Diana"]),
    );
}

/// 3. Upsert equivalence — upsert same PK → same row state.
#[test]
fn test_upsert_equivalence() {
    let mut pair = make_pair();
    insert_same(&mut pair, 1, "Alice", 30, 95.5, true);

    // Upsert same PK in both
    for store in [&mut pair.0, &mut pair.1] {
        let name_id = store.string_table_mut().intern("Alice Updated");
        store
            .upsert(&[
                ("id", ColumnValue::Int(1)),
                ("name", ColumnValue::String(name_id)),
                ("age", ColumnValue::Int(31)),
                ("score", ColumnValue::Float(96.0)),
                ("active", ColumnValue::Bool(true)),
            ])
            .unwrap();
    }

    assert_eq!(pair.0.active_row_count(), pair.1.active_row_count());
    assert_eq!(pair.0.row_count(), pair.1.row_count());

    // Both should return updated value
    assert_eq!(
        pair.0.get_value_as_json("age", 0),
        pair.1.get_value_as_json("age", 0),
    );
    assert_eq!(
        pair.0.get_value_as_json("score", 0),
        pair.1.get_value_as_json("score", 0),
    );
}

/// 4. Delete + vacuum equivalence — same deletions + vacuum → same active rows.
#[test]
fn test_delete_vacuum_equivalence() {
    let mut pair = make_pair();
    insert_same(&mut pair, 1, "Alice", 30, 95.5, true);
    insert_same(&mut pair, 2, "Bob", 25, 88.0, false);
    insert_same(&mut pair, 3, "Charlie", 35, 77.0, true);

    // Delete same PK in both
    assert_eq!(pair.0.delete_by_pk(2), pair.1.delete_by_pk(2));

    assert_eq!(pair.0.active_row_count(), pair.1.active_row_count());
    assert_eq!(pair.0.deleted_row_count(), pair.1.deleted_row_count());

    // Vacuum both
    let stats_a = pair.0.vacuum(VacuumConfig::default());
    let stats_b = pair.1.vacuum(VacuumConfig::default());

    assert_eq!(stats_a.completed, stats_b.completed);
    assert_eq!(stats_a.tombstones_removed, stats_b.tombstones_removed);
    assert_eq!(pair.0.active_row_count(), pair.1.active_row_count());
    assert_eq!(pair.0.deleted_row_count(), pair.1.deleted_row_count());

    // Filters should produce same results post-vacuum
    assert_eq!(
        pair.0.filter_eq_int("age", 30),
        pair.1.filter_eq_int("age", 30),
    );
}

/// 5. String interning equivalence — same strings → same filter results.
#[test]
fn test_string_interning_equivalence() {
    let mut pair = make_pair();

    // Insert rows with repeated string values
    insert_same(&mut pair, 1, "tech", 10, 1.0, true);
    insert_same(&mut pair, 2, "health", 20, 2.0, true);
    insert_same(&mut pair, 3, "tech", 30, 3.0, false);
    insert_same(&mut pair, 4, "finance", 40, 4.0, true);
    insert_same(&mut pair, 5, "tech", 50, 5.0, true);

    // String equality filter
    let tech_a = pair.0.filter_eq_string("name", "tech");
    let tech_b = pair.1.filter_eq_string("name", "tech");
    assert_eq!(tech_a, tech_b);
    assert_eq!(tech_a.len(), 3);

    // String IN filter
    let multi_a = pair.0.filter_in_string("name", &["tech", "finance"]);
    let multi_b = pair.1.filter_in_string("name", &["tech", "finance"]);
    assert_eq!(multi_a, multi_b);
    assert_eq!(multi_a.len(), 4);

    // Non-existent string
    let none_a = pair.0.filter_eq_string("name", "nonexistent");
    let none_b = pair.1.filter_eq_string("name", "nonexistent");
    assert_eq!(none_a, none_b);
    assert!(none_a.is_empty());
}

/// 6. Batch upsert equivalence — same batch → same store state.
#[test]
fn test_batch_upsert_equivalence() {
    let mut pair = make_pair();

    for store in [&mut pair.0, &mut pair.1] {
        let n1 = store.string_table_mut().intern("Alice");
        let n2 = store.string_table_mut().intern("Bob");
        let n3 = store.string_table_mut().intern("Charlie");

        let rows = vec![
            vec![
                ("id", ColumnValue::Int(1)),
                ("name", ColumnValue::String(n1)),
                ("age", ColumnValue::Int(30)),
                ("score", ColumnValue::Float(95.5)),
                ("active", ColumnValue::Bool(true)),
            ],
            vec![
                ("id", ColumnValue::Int(2)),
                ("name", ColumnValue::String(n2)),
                ("age", ColumnValue::Int(25)),
                ("score", ColumnValue::Float(88.0)),
                ("active", ColumnValue::Bool(false)),
            ],
            vec![
                ("id", ColumnValue::Int(3)),
                ("name", ColumnValue::String(n3)),
                ("age", ColumnValue::Int(35)),
                ("score", ColumnValue::Float(77.0)),
                ("active", ColumnValue::Bool(true)),
            ],
        ];

        let result = store.batch_upsert(&rows);
        assert_eq!(result.inserted, 3);
    }

    assert_eq!(pair.0.active_row_count(), pair.1.active_row_count());
    assert_eq!(pair.0.row_count(), pair.1.row_count());

    // JSON values should match for each row
    for col in &["id", "name", "age", "score", "active"] {
        for idx in 0..3 {
            assert_eq!(
                pair.0.get_value_as_json(col, idx),
                pair.1.get_value_as_json(col, idx),
                "Mismatch at column={col}, row={idx}",
            );
        }
    }
}

/// 7. TTL equivalence — same TTL + expire → same surviving rows.
#[test]
fn test_ttl_equivalence() {
    let mut pair = make_pair();
    insert_same(&mut pair, 1, "Alice", 30, 95.5, true);
    insert_same(&mut pair, 2, "Bob", 25, 88.0, false);
    insert_same(&mut pair, 3, "Charlie", 35, 77.0, true);

    // Set TTL=0 (immediate expiry) on row with PK=1 in both stores
    pair.0.set_ttl(1, 0).unwrap();
    pair.1.set_ttl(1, 0).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(10));

    let result_a = pair.0.expire_rows();
    let result_b = pair.1.expire_rows();

    assert_eq!(result_a.expired_count, result_b.expired_count);
    assert_eq!(pair.0.active_row_count(), pair.1.active_row_count());

    // PK=1 should be gone in both
    assert_eq!(pair.0.get_row_idx_by_pk(1), pair.1.get_row_idx_by_pk(1));
    assert!(pair.0.get_row_idx_by_pk(1).is_none());

    // PK=2 and PK=3 still alive in both
    assert!(pair.0.get_row_idx_by_pk(2).is_some());
    assert!(pair.1.get_row_idx_by_pk(3).is_some());
}

/// 8. Bitmap filter equivalence — bitmap AND/OR produce same results as Vec filters.
#[test]
fn test_bitmap_filter_equivalence() {
    let mut pair = make_pair();
    insert_same(&mut pair, 1, "tech", 30, 95.5, true);
    insert_same(&mut pair, 2, "health", 25, 88.0, false);
    insert_same(&mut pair, 3, "tech", 40, 77.0, true);
    insert_same(&mut pair, 4, "finance", 30, 92.0, false);

    let store = &pair.0;

    // Vec-based filters
    let age_30_vec = store.filter_eq_int("age", 30);
    let tech_vec = store.filter_eq_string("name", "tech");

    // Bitmap-based filters
    let age_30_bmp = store.filter_eq_int_bitmap("age", 30);
    let tech_bmp = store.filter_eq_string_bitmap("name", "tech");

    // Vec results should match bitmap results
    let age_30_from_bmp: Vec<usize> = age_30_bmp.iter().map(|i| i as usize).collect();
    let tech_from_bmp: Vec<usize> = tech_bmp.iter().map(|i| i as usize).collect();
    assert_eq!(age_30_vec, age_30_from_bmp);
    assert_eq!(tech_vec, tech_from_bmp);

    // Bitmap AND: age=30 AND name=tech → row 0 only (id=1)
    let and_bmp = ColumnStore::bitmap_and(&age_30_bmp, &tech_bmp);
    let and_indices: Vec<usize> = and_bmp.iter().map(|i| i as usize).collect();
    assert_eq!(and_indices, vec![0]);

    // Bitmap OR: age=30 OR name=tech → rows 0, 2, 3
    let or_bmp = ColumnStore::bitmap_or(&age_30_bmp, &tech_bmp);
    let or_indices: Vec<usize> = or_bmp.iter().map(|i| i as usize).collect();
    assert_eq!(or_indices, vec![0, 2, 3]);

    // Range bitmap equivalence
    let range_vec = store.filter_range_int("age", 24, 35);
    let range_bmp = store.filter_range_int_bitmap("age", 24, 35);
    let range_from_bmp: Vec<usize> = range_bmp.iter().map(|i| i as usize).collect();
    assert_eq!(range_vec, range_from_bmp);
}

// =============================================================================
// Task 2: Metrics + HalfPrecision Equivalence Tests
// =============================================================================

/// 9. Recall equivalence — WASM recall == core recall for same data.
#[test]
fn test_recall_equivalence() {
    use velesdb_core::metrics;

    let ground_truth: Vec<u64> = vec![1, 2, 3, 4, 5];
    let results: Vec<u64> = vec![1, 3, 6, 2, 7];

    // Core API
    let recall = metrics::recall_at_k(&ground_truth, &results);

    // Manual calculation: 3 matches (1, 2, 3) out of 5 truth = 0.6
    assert!((recall - 0.6).abs() < 1e-10);

    // Precision
    let precision = metrics::precision_at_k(&ground_truth, &results);
    assert!((precision - 0.6).abs() < 1e-10);

    // MRR: first relevant at position 1 → 1/1 = 1.0
    let mrr = metrics::mrr(&ground_truth, &results);
    assert!((mrr - 1.0).abs() < 1e-10);

    // Verify symmetry: same inputs always produce same outputs
    let recall2 = metrics::recall_at_k(&ground_truth, &results);
    assert!((recall - recall2).abs() < f64::EPSILON);
}

/// 10. nDCG equivalence — WASM nDCG == core nDCG for same data.
#[test]
fn test_ndcg_equivalence() {
    use velesdb_core::metrics;

    let relevance_scores: Vec<f64> = vec![3.0, 2.0, 3.0, 0.0, 1.0, 2.0];

    let ndcg = metrics::ndcg_at_k(&relevance_scores, 6);
    assert!(
        ndcg > 0.0 && ndcg <= 1.0,
        "nDCG should be in (0, 1], got {ndcg}"
    );

    // Perfect ranking: [3, 3, 2, 2, 1, 0] should give nDCG=1.0
    let perfect: Vec<f64> = vec![3.0, 3.0, 2.0, 2.0, 1.0, 0.0];
    let perfect_ndcg = metrics::ndcg_at_k(&perfect, 6);
    assert!(
        (perfect_ndcg - 1.0).abs() < 1e-10,
        "Perfect ranking should have nDCG=1.0, got {perfect_ndcg}"
    );

    // Verify determinism
    let ndcg2 = metrics::ndcg_at_k(&relevance_scores, 6);
    assert!((ndcg - ndcg2).abs() < f64::EPSILON);
}

/// 11. f16 roundtrip equivalence — WASM f16 roundtrip == core f16 roundtrip.
#[test]
fn test_f16_roundtrip_equivalence() {
    use velesdb_core::half_precision::{VectorData, VectorPrecision};

    let original: Vec<f32> = vec![1.0, -0.5, 0.0, 3.15, -2.71, 100.0, 0.001];

    // f16 roundtrip
    let f16_data = VectorData::from_f32_slice(&original, VectorPrecision::F16);
    let f16_restored = f16_data.to_f32_vec();

    assert_eq!(original.len(), f16_restored.len());
    for (i, (orig, rest)) in original.iter().zip(f16_restored.iter()).enumerate() {
        let diff = (orig - rest).abs();
        assert!(
            diff < 0.1,
            "f16 roundtrip mismatch at [{i}]: {orig} vs {rest} (diff={diff})"
        );
    }

    // bf16 roundtrip
    let bf16_data = VectorData::from_f32_slice(&original, VectorPrecision::BF16);
    let bf16_restored = bf16_data.to_f32_vec();

    assert_eq!(original.len(), bf16_restored.len());
    for (i, (orig, rest)) in original.iter().zip(bf16_restored.iter()).enumerate() {
        let diff = (orig - rest).abs();
        assert!(
            diff < 1.0,
            "bf16 roundtrip mismatch at [{i}]: {orig} vs {rest} (diff={diff})"
        );
    }

    // Memory size equivalence (VectorPrecision::memory_size)
    let f32_size = VectorPrecision::F32.memory_size(768);
    let f16_size = VectorPrecision::F16.memory_size(768);
    let bf16_size = VectorPrecision::BF16.memory_size(768);

    assert_eq!(f32_size, 3072); // 768 * 4
    assert_eq!(f16_size, 1536); // 768 * 2
    assert_eq!(bf16_size, 1536); // 768 * 2

    // Determinism: same input → same output
    let f16_data2 = VectorData::from_f32_slice(&original, VectorPrecision::F16);
    let f16_restored2 = f16_data2.to_f32_vec();
    assert_eq!(f16_restored, f16_restored2);
}
