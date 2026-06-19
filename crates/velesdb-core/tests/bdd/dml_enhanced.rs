//! BDD tests for `VelesQL` v3.5 Phase 4: multi-row INSERT, UPSERT, WITH (quality=...).
//!
//! All tests exercise the full pipeline: SQL string -> parse -> execute -> verify.
//! Requires `persistence` feature (gated in `bdd.rs`).

use super::helpers::{create_test_db, execute_sql, execute_sql_with_params, vector_param};
use std::collections::HashMap;

// =========================================================================
// Helper: create a 4-dim vector collection via SQL for DML tests.
// =========================================================================

fn setup_dml_collection(db: &velesdb_core::Database) {
    execute_sql(
        db,
        "CREATE COLLECTION docs (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE docs should succeed");
}

// =========================================================================
// A. Multi-row INSERT
// =========================================================================

#[test]
fn test_multi_row_insert_creates_all_points() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let mut params = HashMap::new();
    params.insert("v1".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));
    params.insert("v2".to_string(), serde_json::json!([0.0, 1.0, 0.0, 0.0]));
    params.insert("v3".to_string(), serde_json::json!([0.0, 0.0, 1.0, 0.0]));

    execute_sql_with_params(
        &db,
        "INSERT INTO docs (id, vector, title) VALUES (1, $v1, 'A'), (2, $v2, 'B'), (3, $v3, 'C')",
        &params,
    )
    .expect("multi-row INSERT should succeed");

    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 10").expect("SELECT should work");
    assert_eq!(results.len(), 3, "All 3 rows should exist");

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
}

#[test]
fn test_multi_row_insert_with_vectors() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let mut params = HashMap::new();
    params.insert("v1".to_string(), serde_json::json!([0.5, 0.5, 0.0, 0.0]));
    params.insert("v2".to_string(), serde_json::json!([0.0, 0.5, 0.5, 0.0]));

    execute_sql_with_params(
        &db,
        "INSERT INTO docs (id, vector, tag) VALUES (10, $v1, 'alpha'), (11, $v2, 'beta')",
        &params,
    )
    .expect("multi-row INSERT with vectors should succeed");

    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 10").expect("SELECT");
    assert_eq!(results.len(), 2, "Both rows should exist");

    let by_id: HashMap<u64, &velesdb_core::SearchResult> =
        results.iter().map(|r| (r.point.id, r)).collect();

    let r10 = by_id.get(&10).expect("id=10 should exist");
    let r11 = by_id.get(&11).expect("id=11 should exist");

    // payload tag round-trips per row (guards swapped rows / dropped payload)
    assert_eq!(super::helpers::payload_str(r10, "tag"), Some("alpha"));
    assert_eq!(super::helpers::payload_str(r11, "tag"), Some("beta"));

    // stored vectors round-trip (storage path is non-normalizing)
    assert_eq!(r10.point.vector, vec![0.5, 0.5, 0.0, 0.0]);
    assert_eq!(r11.point.vector, vec![0.0, 0.5, 0.5, 0.0]);
}

#[test]
fn test_multi_row_insert_large_batch() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    // Build 20 rows dynamically
    let mut value_parts = Vec::new();
    let mut params = HashMap::new();
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        let pname = format!("v{i}");
        params.insert(pname.clone(), serde_json::json!([fi / 20.0, 1.0, 0.0, 0.0]));
        value_parts.push(format!("({i}, ${pname}, 'doc{i}')"));
    }
    let sql = format!(
        "INSERT INTO docs (id, vector, title) VALUES {}",
        value_parts.join(", ")
    );

    execute_sql_with_params(&db, &sql, &params).expect("20-row INSERT should succeed");

    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 50").expect("SELECT");
    assert_eq!(results.len(), 20, "All 20 rows should exist");
}

// =========================================================================
// B. UPSERT
// =========================================================================

#[test]
fn test_upsert_creates_new_point() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let mut params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

    execute_sql_with_params(
        &db,
        "UPSERT INTO docs (id, vector, title) VALUES (1, $v, 'New')",
        &params,
    )
    .expect("UPSERT should succeed");

    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 10").expect("SELECT");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_upsert_updates_existing_point() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

    // Insert first
    execute_sql_with_params(
        &db,
        "INSERT INTO docs (id, vector, title) VALUES (1, $v, 'Original')",
        &params,
    )
    .expect("INSERT");

    // UPSERT to update
    execute_sql_with_params(
        &db,
        "UPSERT INTO docs (id, vector, title) VALUES (1, $v, 'Updated')",
        &params,
    )
    .expect("UPSERT should succeed");

    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 10").expect("SELECT");
    assert_eq!(
        results.len(),
        1,
        "Should still have 1 point (upserted, not duplicated)"
    );
    let payload = results[0].point.payload.as_ref().expect("payload");
    assert_eq!(
        payload.get("title").and_then(|v| v.as_str()),
        Some("Updated"),
        "Title should be updated by UPSERT"
    );
}

#[test]
fn test_upsert_multi_row_mixed() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let mut params = HashMap::new();
    params.insert("v1".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));
    params.insert("v2".to_string(), serde_json::json!([0.0, 1.0, 0.0, 0.0]));
    params.insert("v3".to_string(), serde_json::json!([0.0, 0.0, 1.0, 0.0]));

    // Insert id=1
    execute_sql_with_params(
        &db,
        "INSERT INTO docs (id, vector, title) VALUES (1, $v1, 'Original')",
        &params,
    )
    .expect("INSERT");

    // UPSERT 3 rows: id=1 (existing), id=2 (new), id=3 (new)
    execute_sql_with_params(
        &db,
        "UPSERT INTO docs (id, vector, title) VALUES (1, $v1, 'Updated1'), (2, $v2, 'New2'), (3, $v3, 'New3')",
        &params,
    )
    .expect("multi-row UPSERT should succeed");

    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 10").expect("SELECT");
    assert_eq!(results.len(), 3, "Should have 3 unique points");

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(
        ids.contains(&1) && ids.contains(&2) && ids.contains(&3),
        "ids 1,2,3 must all exist"
    );

    let point1 = results
        .iter()
        .find(|r| r.point.id == 1)
        .expect("id=1 should exist");
    let title1 = point1
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("title"))
        .and_then(|v| v.as_str());
    assert_eq!(
        title1,
        Some("Updated1"),
        "id=1 must be UPDATED in place, not left as 'Original'"
    );
}

// =========================================================================
// C. WITH (quality='...')
// =========================================================================

#[test]
fn test_search_with_quality_fast() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    // Insert some points via the API to have data for search
    let vc = db
        .get_vector_collection("docs")
        .expect("docs collection should exist");
    let mut points = Vec::new();
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        points.push(velesdb_core::Point::new(
            i,
            vec![fi / 20.0, 1.0 - fi / 20.0, 0.5, 0.3],
            Some(serde_json::json!({ "idx": i })),
        ));
    }
    vc.upsert(points).expect("upsert");

    let params = vector_param(&[0.5, 0.5, 0.5, 0.3]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (quality = 'fast')",
        &params,
    )
    .expect("WITH quality='fast' should succeed");
    assert_eq!(results.len(), 5);

    // The 5 hits must be 5 distinct ids drawn from the 20 inserted points,
    // returned in non-increasing score order. `len()==5` alone would not
    // catch duplicate ids or an unsorted result set from a broken NEAR path.
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids.len(), 5, "all 5 NEAR hits must be distinct ids");
    assert!(
        results.iter().all(|r| r.point.id < 20),
        "every hit must be one of the 20 inserted points"
    );
    assert!(
        results.windows(2).all(|w| w[0].score >= w[1].score),
        "NEAR results must be sorted by non-increasing score"
    );
}

#[test]
fn test_search_with_quality_accurate() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let vc = db.get_vector_collection("docs").expect("docs");
    let mut points = Vec::new();
    for i in 0u64..20 {
        #[allow(clippy::cast_precision_loss)]
        let fi = i as f32;
        points.push(velesdb_core::Point::new(
            i,
            vec![fi / 20.0, 1.0 - fi / 20.0, 0.5, 0.3],
            Some(serde_json::json!({ "idx": i })),
        ));
    }
    vc.upsert(points).expect("upsert");

    let params = vector_param(&[0.5, 0.5, 0.5, 0.3]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (quality = 'accurate')",
        &params,
    )
    .expect("WITH quality='accurate' should succeed");
    assert_eq!(results.len(), 5);

    // The 5 hits must be 5 distinct ids drawn from the 20 inserted points,
    // returned in non-increasing score order. `len()==5` alone would not
    // catch duplicate ids or an unsorted result set from a broken NEAR path.
    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids.len(), 5, "all 5 NEAR hits must be distinct ids");
    assert!(
        results.iter().all(|r| r.point.id < 20),
        "every hit must be one of the 20 inserted points"
    );
    assert!(
        results.windows(2).all(|w| w[0].score >= w[1].score),
        "NEAR results must be sorted by non-increasing score"
    );
}

// =========================================================================
// D. Complex / lifecycle
// =========================================================================

#[test]
fn test_insert_upsert_lifecycle() {
    let (_dir, db) = create_test_db();
    setup_dml_collection(&db);

    let mut params = HashMap::new();
    params.insert("v1".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));
    params.insert("v2".to_string(), serde_json::json!([0.0, 1.0, 0.0, 0.0]));
    params.insert("v3".to_string(), serde_json::json!([0.0, 0.0, 1.0, 0.0]));

    // Step 1: INSERT 2 rows
    execute_sql_with_params(
        &db,
        "INSERT INTO docs (id, vector, title) VALUES (1, $v1, 'First'), (2, $v2, 'Second')",
        &params,
    )
    .expect("INSERT 2 rows");

    // Step 2: UPSERT id=1 (update) + id=3 (new)
    execute_sql_with_params(
        &db,
        "UPSERT INTO docs (id, vector, title) VALUES (1, $v1, 'FirstUpdated'), (3, $v3, 'Third')",
        &params,
    )
    .expect("UPSERT 2 rows");

    // Step 3: Verify final state
    let results = execute_sql(&db, "SELECT * FROM docs LIMIT 10").expect("SELECT");
    assert_eq!(results.len(), 3, "Should have 3 total points: id 1, 2, 3");

    // Verify id=1 has updated payload
    let point1 = results
        .iter()
        .find(|r| r.point.id == 1)
        .expect("id=1 should exist");
    let title = point1
        .point
        .payload
        .as_ref()
        .and_then(|p| p.get("title"))
        .and_then(|v| v.as_str());
    assert_eq!(title, Some("FirstUpdated"), "id=1 title should be updated");
}

// =========================================================================
// E. Negative
// =========================================================================

#[test]
fn test_upsert_nonexistent_collection_fails() {
    let (_dir, db) = create_test_db();

    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));

    let result = execute_sql_with_params(
        &db,
        "UPSERT INTO nonexistent (id, vector) VALUES (1, $v)",
        &params,
    );
    assert!(
        result.is_err(),
        "UPSERT into nonexistent collection should fail"
    );
}
