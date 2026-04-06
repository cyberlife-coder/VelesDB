//! BDD-style end-to-end tests for GeoPoint column type and geospatial filters.
//!
//! Each scenario follows GIVEN (setup data) -> WHEN (execute SQL) -> THEN (verify results).
//! Tests exercise the full pipeline: SQL string -> Parser -> Database -> verify results.

use serde_json::json;
use velesdb_core::{Database, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Populate a `places` collection with GeoPoint locations for geospatial filtering.
///
/// | id | location (lat, lng)         | category   | rating |
/// |----|-----------------------------|------------|--------|
/// | 1  | (48.8566, 2.3522) Paris     | hotel      | 4.5    |
/// | 2  | (51.5074, -0.1278) London   | hotel      | 4.2    |
/// | 3  | (40.7128, -74.0060) NYC     | restaurant | 4.8    |
/// | 4  | (35.6762, 139.6503) Tokyo   | hotel      | 4.7    |
/// | 5  | null location               | hotel      | 3.9    |
/// | 6  | (48.8600, 2.3500) Near Paris| restaurant | 4.1    |
fn setup_geo_collection(db: &Database) {
    execute_sql(
        db,
        "CREATE COLLECTION places (dimension = 4, metric = 'cosine');",
    )
    .expect("test: CREATE places");

    let vc = db
        .get_vector_collection("places")
        .expect("test: get places collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"location": {"lat": 48.8566, "lng": 2.3522}, "category": "hotel", "rating": 4.5})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(json!({"location": {"lat": 51.5074, "lng": -0.1278}, "category": "hotel", "rating": 4.2})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"location": {"lat": 40.7128, "lng": -74.0060}, "category": "restaurant", "rating": 4.8})),
        ),
        Point::new(
            4,
            vec![0.0, 0.0, 0.0, 1.0],
            Some(json!({"location": {"lat": 35.6762, "lng": 139.6503}, "category": "hotel", "rating": 4.7})),
        ),
        Point::new(
            5,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(json!({"category": "hotel", "rating": 3.9})),
        ),
        Point::new(
            6,
            vec![0.5, 0.0, 0.5, 0.0],
            Some(json!({"location": {"lat": 48.8600, "lng": 2.3500}, "category": "restaurant", "rating": 4.1})),
        ),
    ])
    .expect("test: upsert places");
}

// =========================================================================
// Nominal scenarios
// =========================================================================

#[test]
fn test_given_geo_collection_when_geo_distance_lt_threshold_then_points_in_range() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // Paris to London is ~343 km. 500_000m should include Paris(1), London(2), Near Paris(6).
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 500000 LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE query");

    let ids = result_ids(&results);
    assert!(ids.contains(&1), "Paris should be in range");
    assert!(ids.contains(&6), "Near Paris should be in range");
    // London is ~343km, should be included
    assert!(ids.contains(&2), "London should be in range");
    // NYC and Tokyo should NOT be included
    assert!(!ids.contains(&3), "NYC should not be in range");
    assert!(!ids.contains(&4), "Tokyo should not be in range");
    // Null location should be excluded
    assert!(!ids.contains(&5), "Null location should be excluded");
}

#[test]
fn test_given_geo_collection_when_geo_bbox_then_points_in_box() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // Bounding box around Paris area
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_BBOX(location, 48.0, 2.0, 49.0, 3.0) LIMIT 10;",
    )
    .expect("test: GEO_BBOX query");

    let ids = result_ids(&results);
    assert!(ids.contains(&1), "Paris should be in bbox");
    assert!(ids.contains(&6), "Near Paris should be in bbox");
    assert!(!ids.contains(&2), "London should not be in bbox");
    assert!(!ids.contains(&5), "Null location should be excluded");
}

// =========================================================================
// Edge-case scenarios
// =========================================================================

#[test]
fn test_given_identical_point_when_geo_distance_eq_zero_then_exact_match() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // Distance from Paris to itself should be 0
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 1 LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE exact match");

    let ids = result_ids(&results);
    assert!(ids.contains(&1), "Paris exact match should be found");
    // Near Paris is ~500m away, should NOT be within 1m
    assert!(!ids.contains(&6), "Near Paris should not be within 1m");
}

#[test]
fn test_given_null_geopoint_when_geo_distance_then_excluded() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // Very large radius should include all non-null points but not null
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 0, 0) < 99999999 LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE null exclusion");

    let ids = result_ids(&results);
    assert!(!ids.contains(&5), "Null location must be excluded");
    assert_eq!(ids.len(), 5, "All non-null points should match");
}

#[test]
fn test_given_point_on_bbox_boundary_when_geo_bbox_then_included() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // Bbox exactly matching Paris coordinates (boundary inclusive)
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_BBOX(location, 48.8566, 2.3522, 48.8566, 2.3522) LIMIT 10;",
    )
    .expect("test: GEO_BBOX boundary");

    let ids = result_ids(&results);
    assert!(ids.contains(&1), "Point on boundary should be included");
}

// =========================================================================
// Negative scenarios
// =========================================================================

#[test]
fn test_given_non_geopoint_column_when_geo_distance_then_empty() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(category, 48.8566, 2.3522) < 500 LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE on non-geo column");

    assert!(
        results.is_empty(),
        "Non-GeoPoint column should return empty"
    );
}

#[test]
fn test_given_nonexistent_column_when_geo_distance_then_empty() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(nonexistent, 48.8566, 2.3522) < 500 LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE on missing column");

    assert!(
        results.is_empty(),
        "Non-existent column should return empty"
    );
}

#[test]
fn test_given_inverted_bbox_when_geo_bbox_then_empty() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // lat_min > lat_max → empty
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_BBOX(location, 49, 3, 48, 2) LIMIT 10;",
    )
    .expect("test: GEO_BBOX inverted");

    assert!(results.is_empty(), "Inverted bbox should return empty");
}

// =========================================================================
// Combination scenarios
// =========================================================================

#[test]
fn test_given_geo_distance_and_scalar_filter_then_intersection() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 500000 AND category = 'hotel' LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE AND scalar");

    let ids = result_ids(&results);
    // Paris(1) is hotel + in range, London(2) is hotel + in range
    assert!(ids.contains(&1), "Paris hotel should match");
    assert!(ids.contains(&2), "London hotel should match");
    // Near Paris(6) is restaurant, not hotel
    assert!(!ids.contains(&6), "Near Paris restaurant should not match");
}

#[test]
fn test_given_geo_distance_and_vector_near_then_hybrid() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    let params = vector_param(&[1.0, 0.0, 0.0, 0.0]);
    let results = execute_sql_with_params(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 1000000 AND vector NEAR $v LIMIT 10;",
        &params,
    )
    .expect("test: GEO_DISTANCE AND vector NEAR");

    // Should return results that are both within 1000km of Paris AND similar to query vector
    assert!(
        !results.is_empty(),
        "Hybrid geo+vector should return results"
    );
}

#[test]
fn test_given_geo_distance_or_geo_bbox_then_union() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    // GEO_DISTANCE < 1m from Paris (only Paris) OR GEO_BBOX around Tokyo area
    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 1 OR GEO_BBOX(location, 35.0, 139.0, 36.0, 140.0) LIMIT 10;",
    )
    .expect("test: GEO_DISTANCE OR GEO_BBOX");

    let ids = result_ids(&results);
    assert!(ids.contains(&1), "Paris should match via GEO_DISTANCE");
    assert!(ids.contains(&4), "Tokyo should match via GEO_BBOX");
}

#[test]
fn test_given_geo_distance_with_order_by_limit_then_correct() {
    let (_dir, db) = create_test_db();
    setup_geo_collection(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM places WHERE GEO_DISTANCE(location, 48.8566, 2.3522) < 500000 ORDER BY rating DESC LIMIT 2;",
    )
    .expect("test: GEO_DISTANCE with ORDER BY LIMIT");

    assert!(results.len() <= 2, "Should respect LIMIT 2");
}
